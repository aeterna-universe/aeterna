use async_trait::async_trait;
use git2::{Repository, Signature};
use mk_core::traits::KnowledgeRepository;
use mk_core::types::{KnowledgeEntry, KnowledgeLayer, KnowledgeStatus, KnowledgeType};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use walkdir::WalkDir;

use crate::git_provider::{GitProvider, GovernanceBranch, WriteOperation, requires_governance};

#[derive(Error, Debug)]
pub enum RepositoryError {
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid path: {0}")]
    InvalidPath(String),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Remote error: {0}")]
    Remote(String),
}

pub struct RemoteConfig {
    pub url: String,
    pub branch: String,
    pub git_provider: Option<Arc<dyn GitProvider>>,
}

pub struct GitRepository {
    root_path: PathBuf,
    remote_url: Option<String>,
    branch: String,
    git_provider: Option<Arc<dyn GitProvider>>,
    rw_lock: Arc<RwLock<()>>,
}

impl GitRepository {
    pub fn new(root_path: impl Into<PathBuf>) -> Result<Self, RepositoryError> {
        Self::new_with_remote(root_path, None)
    }

    pub fn new_with_remote(
        root_path: impl Into<PathBuf>,
        remote_config: Option<RemoteConfig>,
    ) -> Result<Self, RepositoryError> {
        let root_path = root_path.into();
        if !root_path.exists() {
            std::fs::create_dir_all(&root_path)?;
        }

        let (remote_url, branch, git_provider) = if let Some(cfg) = remote_config {
            let url = if cfg.git_provider.is_some() {
                Self::ssh_url_to_https(&cfg.url)
            } else {
                cfg.url
            };
            (Some(url), cfg.branch, cfg.git_provider)
        } else {
            (None, "main".to_string(), None)
        };

        let repo = Self {
            root_path,
            remote_url,
            branch,
            git_provider,
            rw_lock: Arc::new(RwLock::new(())),
        };

        if repo.remote_url.is_some() {
            repo.init_or_sync_remote()?;
        } else if Repository::open(&repo.root_path).is_err() {
            Repository::init(&repo.root_path)?;
        }

        Ok(repo)
    }

    fn init_or_sync_remote(&self) -> Result<(), RepositoryError> {
        let git_dir = self.root_path.join(".git");
        if git_dir.exists() {
            let repo = Repository::open(&self.root_path)?;
            self.ensure_origin_remote(&repo)?;
            self.pull_from_remote()?;
            return Ok(());
        }

        if Self::is_dir_empty(&self.root_path)? {
            self.clone_from_remote()?;
            return Ok(());
        }

        Err(RepositoryError::Remote(
            "Repository path is not empty and is not a git repository".to_string(),
        ))
    }

    fn is_dir_empty(path: &Path) -> Result<bool, RepositoryError> {
        let mut entries = std::fs::read_dir(path)?;
        Ok(entries.next().is_none())
    }

    fn clone_from_remote(&self) -> Result<(), RepositoryError> {
        let remote_url = self
            .remote_url
            .as_ref()
            .ok_or_else(|| RepositoryError::Remote("Remote URL not configured".to_string()))?;

        let mut fetch_options = git2::FetchOptions::new();
        fetch_options.remote_callbacks(self.remote_callbacks());

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch_options);
        builder.branch(&self.branch);
        builder
            .clone(remote_url, &self.root_path)
            .map_err(RepositoryError::from)?;
        Ok(())
    }

    fn ensure_origin_remote(&self, repo: &Repository) -> Result<(), RepositoryError> {
        let expected_url = self
            .remote_url
            .as_ref()
            .ok_or_else(|| RepositoryError::Remote("Remote URL not configured".to_string()))?;

        match repo.find_remote("origin") {
            Ok(remote) => {
                let actual_url = remote.url().unwrap_or_default();
                if actual_url != expected_url {
                    return Err(RepositoryError::Remote(format!(
                        "Remote mismatch: expected '{expected_url}', found '{actual_url}'"
                    )));
                }
            }
            Err(_) => {
                repo.remote("origin", expected_url)?;
            }
        }

        Ok(())
    }

    fn merge_remote_into_local(&self) -> Result<(), RepositoryError> {
        let repo = Repository::open(&self.root_path)?;
        let mut remote = repo.find_remote("origin")?;

        let mut fetch_options = git2::FetchOptions::new();
        fetch_options.remote_callbacks(self.remote_callbacks());
        remote.fetch(&[self.branch.as_str()], Some(&mut fetch_options), None)?;

        let fetch_head = repo.find_reference("FETCH_HEAD")?;
        let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;
        let (analysis, _) = repo.merge_analysis(&[&fetch_commit])?;

        if analysis.is_up_to_date() {
            return Ok(());
        }

        if analysis.is_fast_forward() {
            self.fast_forward_to_fetch(&repo, &fetch_commit)?;
            return Ok(());
        }

        if analysis.is_normal() {
            repo.merge(&[&fetch_commit], None, None)?;
            let mut index = repo.index()?;
            if index.has_conflicts() {
                repo.cleanup_state()?;
                return Err(RepositoryError::Remote(
                    "Merge conflict while reconciling with remote".to_string(),
                ));
            }

            let tree_id = index.write_tree_to(&repo)?;
            let tree = repo.find_tree(tree_id)?;
            let sig = repo
                .signature()
                .or_else(|_| Signature::now("Aeterna", "system@aeterna.ai"))?;

            let head_commit = repo.head()?.peel_to_commit()?;
            let remote_commit = repo.find_commit(fetch_commit.id())?;
            repo.commit(
                Some("HEAD"),
                &sig,
                &sig,
                "Merge remote changes",
                &tree,
                &[&head_commit, &remote_commit],
            )?;

            let mut checkout = git2::build::CheckoutBuilder::new();
            checkout.force();
            repo.checkout_head(Some(&mut checkout))?;
            repo.cleanup_state()?;
            return Ok(());
        }

        Ok(())
    }

    fn fast_forward_to_fetch(
        &self,
        repo: &Repository,
        fetch_commit: &git2::AnnotatedCommit<'_>,
    ) -> Result<(), RepositoryError> {
        let branch_ref = format!("refs/heads/{}", self.branch);

        if let Ok(mut reference) = repo.find_reference(&branch_ref) {
            reference.set_target(fetch_commit.id(), "Fast-forward")?;
        } else {
            repo.reference(&branch_ref, fetch_commit.id(), true, "Create local branch")?;
        }

        repo.set_head(&branch_ref)?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.force();
        repo.checkout_head(Some(&mut checkout))?;
        Ok(())
    }

    fn layer_dir(layer: KnowledgeLayer) -> &'static str {
        match layer {
            KnowledgeLayer::Tenant => "tenant",
            KnowledgeLayer::Org => "org",
            KnowledgeLayer::Team => "team",
            KnowledgeLayer::Project => "project",
        }
    }

    fn ssh_url_to_https(url: &str) -> String {
        if let Some(rest) = url.strip_prefix("git@github.com:") {
            let rest = rest.strip_suffix(".git").unwrap_or(rest);
            format!("https://github.com/{rest}.git")
        } else {
            url.to_string()
        }
    }

    fn remote_callbacks(&self) -> git2::RemoteCallbacks<'_> {
        let mut callbacks = git2::RemoteCallbacks::new();
        if let Some(ref provider) = self.git_provider {
            let provider = Arc::clone(provider);
            callbacks.credentials(move |_url, _username, _allowed| {
                let handle = tokio::runtime::Handle::current();
                let token = std::thread::scope(|s| {
                    s.spawn(|| handle.block_on(provider.get_installation_token()))
                        .join()
                        .expect("token thread panicked")
                })
                .map_err(|e| git2::Error::from_str(&format!("Token fetch failed: {e}")))?;
                git2::Cred::userpass_plaintext("x-access-token", &token)
            });
        }
        callbacks
    }

    pub fn pull_from_remote(&self) -> Result<(), RepositoryError> {
        if self.remote_url.is_none() {
            return Ok(());
        }

        let repo = Repository::open(&self.root_path)?;
        let mut remote = repo.find_remote("origin")?;

        let mut fetch_options = git2::FetchOptions::new();
        fetch_options.remote_callbacks(self.remote_callbacks());
        remote.fetch(&[self.branch.as_str()], Some(&mut fetch_options), None)?;

        let fetch_head = repo.find_reference("FETCH_HEAD")?;
        let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;
        let (analysis, _) = repo.merge_analysis(&[&fetch_commit])?;

        if analysis.is_up_to_date() {
            return Ok(());
        }

        if analysis.is_fast_forward() {
            self.fast_forward_to_fetch(&repo, &fetch_commit)?;
            return Ok(());
        }

        if analysis.is_normal() {
            let target = repo.find_object(fetch_commit.id(), None)?;
            repo.reset(&target, git2::ResetType::Hard, None)?;
        }

        Ok(())
    }

    pub fn push_to_remote(&self) -> Result<(), RepositoryError> {
        if self.remote_url.is_none() {
            return Ok(());
        }

        let refspec = format!("refs/heads/{0}:refs/heads/{0}", self.branch);

        for attempt in 0..3 {
            let repo = Repository::open(&self.root_path)?;
            let mut remote = repo.find_remote("origin")?;
            let mut push_options = git2::PushOptions::new();
            push_options.remote_callbacks(self.remote_callbacks());

            match remote.push(&[refspec.as_str()], Some(&mut push_options)) {
                Ok(()) => return Ok(()),
                Err(err) => {
                    if attempt == 2 {
                        return Err(RepositoryError::Remote(err.to_string()));
                    }
                    self.merge_remote_into_local()?;
                }
            }
        }

        Ok(())
    }

    fn resolve_path(
        &self,
        ctx: &mk_core::types::TenantContext,
        layer: KnowledgeLayer,
        path: &str,
    ) -> PathBuf {
        let layer_dir = Self::layer_dir(layer);
        self.root_path
            .join(ctx.tenant_id.as_str())
            .join(layer_dir)
            .join(path)
    }

    pub fn commit(&self, message: &str) -> Result<String, RepositoryError> {
        let span = tracing::info_span!("knowledge_commit", message = %message);
        let _enter = span.enter();

        let repo = Repository::open(&self.root_path)?;
        let mut index = repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;

        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;

        let sig = repo
            .signature()
            .or_else(|_| Signature::now("Aeterna", "system@aeterna.ai"))?;

        let parent_commit = match repo.head() {
            Ok(head) => Some(head.peel_to_commit()?),
            Err(_) => None,
        };

        let parents = match &parent_commit {
            Some(c) => vec![c],
            None => vec![],
        };

        let commit_id = repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)?;

        Ok(commit_id.to_string())
    }

    pub fn get_head_commit_sync(&self) -> Result<Option<String>, RepositoryError> {
        let repo = Repository::open(&self.root_path)?;
        match repo.head() {
            Ok(head) => Ok(Some(head.peel_to_commit()?.id().to_string())),
            Err(_) => Ok(None),
        }
    }

    pub fn root_path(&self) -> &std::path::Path {
        &self.root_path
    }

    pub fn new_mock() -> Self {
        let root = tempfile::tempdir().unwrap().path().join("mock_repo");
        Self::new(root).unwrap()
    }

    async fn get_inner(
        &self,
        ctx: mk_core::types::TenantContext,
        layer: KnowledgeLayer,
        path: &str,
    ) -> Result<Option<KnowledgeEntry>, RepositoryError> {
        let full_path = self.resolve_path(&ctx, layer, path);
        if !full_path.exists() {
            return Ok(None);
        }

        let metadata_path = full_path.with_extension("metadata.json");

        let content = tokio::fs::read_to_string(&full_path).await?;

        let commit_hash = {
            // Use per-file commit history so the version is the commit that last
            // touched *this file*, not the repo HEAD (which advances with every
            // promotion-request or relation write).
            let repo = Repository::open(&self.root_path)?;
            let relative = full_path
                .strip_prefix(&self.root_path)
                .unwrap_or(&full_path);
            let relative_str = relative.to_string_lossy();
            let mut revwalk = repo.revwalk()?;
            revwalk.set_sorting(git2::Sort::TIME)?;
            revwalk.push_head().ok();
            // Walk commits and find the most recent one touching this file.
            let mut file_commit: Option<String> = None;
            'walk: for oid in revwalk.flatten() {
                if let Ok(commit) = repo.find_commit(oid)
                    && let Ok(tree) = commit.tree()
                    && tree
                        .get_path(std::path::Path::new(relative_str.as_ref()))
                        .is_ok()
                {
                    // Check parent to see if this commit modified the file
                    let parent_has_file = commit
                        .parent(0)
                        .ok()
                        .and_then(|p| p.tree().ok())
                        .map(|pt| {
                            pt.get_path(std::path::Path::new(relative_str.as_ref()))
                                .is_ok()
                        })
                        .unwrap_or(false);
                    // First commit (no parent) or file changed → this is the last touch
                    if commit.parent_count() == 0 || !parent_has_file {
                        file_commit = Some(oid.to_string());
                        break 'walk;
                    }
                    // Compare blob OIDs between parent and this commit
                    if let Ok(parent) = commit.parent(0) {
                        let parent_tree = parent.tree().ok();
                        let cur_entry = tree
                            .get_path(std::path::Path::new(relative_str.as_ref()))
                            .ok();
                        let par_entry = parent_tree.as_ref().and_then(|pt| {
                            pt.get_path(std::path::Path::new(relative_str.as_ref()))
                                .ok()
                        });
                        match (cur_entry, par_entry) {
                            (Some(c), Some(p)) if c.id() != p.id() => {
                                file_commit = Some(oid.to_string());
                                break 'walk;
                            }
                            (Some(_), None) => {
                                file_commit = Some(oid.to_string());
                                break 'walk;
                            }
                            _ => continue,
                        }
                    }
                }
            }
            file_commit
        };

        let (kind, status, summaries, metadata, author, updated_at) = if metadata_path.exists() {
            let meta_content = tokio::fs::read_to_string(&metadata_path).await?;
            let meta: serde_json::Value = serde_json::from_str(&meta_content)?;
            (
                serde_json::from_value(meta["kind"].clone()).unwrap_or(KnowledgeType::Spec),
                serde_json::from_value(meta["status"].clone())
                    .unwrap_or(mk_core::types::KnowledgeStatus::Accepted),
                serde_json::from_value(meta["summaries"].clone()).unwrap_or_default(),
                serde_json::from_value(meta["metadata"].clone()).unwrap_or_default(),
                serde_json::from_value(meta["author"].clone()).unwrap_or_default(),
                meta["updated_at"]
                    .as_i64()
                    .unwrap_or_else(|| chrono::Utc::now().timestamp()),
            )
        } else {
            (
                KnowledgeType::Spec,
                mk_core::types::KnowledgeStatus::Accepted,
                std::collections::HashMap::new(),
                std::collections::HashMap::new(),
                None,
                chrono::Utc::now().timestamp(),
            )
        };

        Ok(Some(KnowledgeEntry {
            path: path.to_string(),
            content,
            layer,
            kind,
            status,
            summaries,
            metadata,
            commit_hash,
            author,
            updated_at,
        }))
    }

    async fn store_inner(
        &self,
        ctx: mk_core::types::TenantContext,
        entry: KnowledgeEntry,
        message: &str,
    ) -> Result<String, RepositoryError> {
        let full_path = self.resolve_path(&ctx, entry.layer, &entry.path);
        let metadata_path = full_path.with_extension("metadata.json");

        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(&full_path, entry.content).await?;

        let meta = serde_json::json!({
            "kind": entry.kind,
            "status": entry.status,
            "summaries": entry.summaries,
            "metadata": entry.metadata,
            "author": entry.author,
            "updated_at": entry.updated_at,
        });
        tokio::fs::write(&metadata_path, serde_json::to_string(&meta)?).await?;

        self.commit(message)
    }

    async fn list_inner(
        &self,
        ctx: mk_core::types::TenantContext,
        layer: KnowledgeLayer,
        prefix: &str,
    ) -> Result<Vec<KnowledgeEntry>, RepositoryError> {
        let tenant_path = self.root_path.join(ctx.tenant_id.as_str());
        let layer_path = tenant_path.join(Self::layer_dir(layer));

        if !layer_path.exists() {
            return Ok(vec![]);
        }

        let mut entries = Vec::new();
        for entry in WalkDir::new(&layer_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_type().is_file() && !e.path().to_string_lossy().ends_with(".metadata.json")
            })
        {
            let path = entry.path();
            let relative_path = path
                .strip_prefix(&layer_path)
                .map_err(|_| RepositoryError::InvalidPath(path.to_string_lossy().into_owned()))?;

            if relative_path.to_string_lossy().starts_with(prefix)
                && let Some(ke) = self
                    .get_inner(ctx.clone(), layer, &relative_path.to_string_lossy())
                    .await?
            {
                entries.push(ke);
            }
        }

        Ok(entries)
    }

    async fn delete_inner(
        &self,
        ctx: mk_core::types::TenantContext,
        layer: KnowledgeLayer,
        path: &str,
        message: &str,
    ) -> Result<String, RepositoryError> {
        let full_path = self.resolve_path(&ctx, layer, path);
        let metadata_path = full_path.with_extension("metadata.json");

        if full_path.exists() {
            tokio::fs::remove_file(full_path).await?;
            if metadata_path.exists() {
                tokio::fs::remove_file(metadata_path).await?;
            }
            self.commit(message)
        } else {
            Ok(String::new())
        }
    }

    async fn search_inner(
        &self,
        ctx: mk_core::types::TenantContext,
        query: &str,
        layers: Vec<KnowledgeLayer>,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>, RepositoryError> {
        let mut results = Vec::new();
        for layer in layers {
            let entries = self.list_inner(ctx.clone(), layer, "").await?;
            for entry in entries {
                if entry.content.contains(query) || entry.path.contains(query) {
                    results.push(entry);
                }
                if results.len() >= limit {
                    return Ok(results);
                }
            }
        }
        Ok(results)
    }

    async fn store_governance_track(
        &self,
        ctx: mk_core::types::TenantContext,
        entry: KnowledgeEntry,
        message: &str,
    ) -> Result<String, RepositoryError> {
        self.store_governance_track_with_verb(ctx, entry, message, "create")
            .await
    }

    async fn store_governance_track_with_verb(
        &self,
        ctx: mk_core::types::TenantContext,
        entry: KnowledgeEntry,
        message: &str,
        verb: &str,
    ) -> Result<String, RepositoryError> {
        let git_provider = self
            .git_provider
            .as_ref()
            .ok_or_else(|| RepositoryError::Remote("Git provider not configured".to_string()))?;

        let branch = GovernanceBranch::new(verb, &entry.path);
        let base_sha = git_provider
            .get_default_branch_sha()
            .await
            .map_err(|e| RepositoryError::Remote(e.to_string()))?;

        if let Err(err) = git_provider.create_branch(&branch.name, &base_sha).await {
            let msg = err.to_string();
            if !msg.contains("Branch already exists") {
                return Err(RepositoryError::Remote(msg));
            }
        }

        let content = serde_json::to_string_pretty(&serde_json::json!({
            "content": entry.content,
            "kind": entry.kind,
            "status": entry.status,
            "metadata": entry.metadata,
            "author": entry.author,
            "updated_at": entry.updated_at,
        }))?;

        let file_path = format!(
            "{}/{}/{}",
            ctx.tenant_id,
            Self::layer_dir(entry.layer),
            entry.path
        );

        git_provider
            .commit_to_branch(&branch.name, &file_path, content.as_bytes(), message)
            .await
            .map_err(|e| RepositoryError::Remote(e.to_string()))?;

        let pr = git_provider
            .create_pull_request(
                &format!("[Governance] {message}"),
                message,
                &branch.name,
                &self.branch,
            )
            .await
            .map_err(|e| RepositoryError::Remote(e.to_string()))?;

        Ok(format!("pr:{}", pr.number))
    }

    async fn delete_governance_track(
        &self,
        ctx: mk_core::types::TenantContext,
        layer: KnowledgeLayer,
        path: &str,
        message: &str,
    ) -> Result<String, RepositoryError> {
        let git_provider = self
            .git_provider
            .as_ref()
            .ok_or_else(|| RepositoryError::Remote("Git provider not configured".to_string()))?;

        let branch = GovernanceBranch::new("delete", path);
        let base_sha = git_provider
            .get_default_branch_sha()
            .await
            .map_err(|e| RepositoryError::Remote(e.to_string()))?;

        if let Err(err) = git_provider.create_branch(&branch.name, &base_sha).await {
            let msg = err.to_string();
            if !msg.contains("Branch already exists") {
                return Err(RepositoryError::Remote(msg));
            }
        }

        let file_path = format!("{}/{}/{}", ctx.tenant_id, Self::layer_dir(layer), path);
        let marker = serde_json::to_string_pretty(&serde_json::json!({
            "deleted": true,
            "path": path,
            "message": message,
        }))?;

        git_provider
            .commit_to_branch(&branch.name, &file_path, marker.as_bytes(), message)
            .await
            .map_err(|e| RepositoryError::Remote(e.to_string()))?;

        let pr = git_provider
            .create_pull_request(
                &format!("[Governance] {message}"),
                message,
                &branch.name,
                &self.branch,
            )
            .await
            .map_err(|e| RepositoryError::Remote(e.to_string()))?;

        Ok(format!("pr:{}", pr.number))
    }

    async fn update_status_local(
        &self,
        ctx: mk_core::types::TenantContext,
        layer: KnowledgeLayer,
        path: &str,
        new_status: KnowledgeStatus,
        message: &str,
    ) -> Result<String, RepositoryError> {
        let mut entry = self
            .get_inner(ctx.clone(), layer, path)
            .await?
            .ok_or_else(|| RepositoryError::InvalidPath(path.to_string()))?;
        entry.status = new_status;
        entry.updated_at = chrono::Utc::now().timestamp();
        self.store_inner(ctx, entry, message).await
    }

    async fn update_status_governance_track(
        &self,
        ctx: mk_core::types::TenantContext,
        layer: KnowledgeLayer,
        path: &str,
        new_status: KnowledgeStatus,
        message: &str,
    ) -> Result<String, RepositoryError> {
        let mut entry = self
            .get_inner(ctx.clone(), layer, path)
            .await?
            .ok_or_else(|| RepositoryError::InvalidPath(path.to_string()))?;
        entry.status = new_status;
        entry.updated_at = chrono::Utc::now().timestamp();
        self.store_governance_track_with_verb(ctx, entry, message, "status")
            .await
    }

    async fn promote_local(
        &self,
        ctx: mk_core::types::TenantContext,
        source_layer: KnowledgeLayer,
        target_layer: KnowledgeLayer,
        path: &str,
        message: &str,
    ) -> Result<String, RepositoryError> {
        let mut entry = self
            .get_inner(ctx.clone(), source_layer, path)
            .await?
            .ok_or_else(|| RepositoryError::InvalidPath(path.to_string()))?;
        entry.layer = target_layer;
        entry.updated_at = chrono::Utc::now().timestamp();
        self.store_inner(ctx, entry, message).await
    }

    async fn promote_governance_track(
        &self,
        ctx: mk_core::types::TenantContext,
        source_layer: KnowledgeLayer,
        target_layer: KnowledgeLayer,
        path: &str,
        message: &str,
    ) -> Result<String, RepositoryError> {
        let mut entry = self
            .get_inner(ctx.clone(), source_layer, path)
            .await?
            .ok_or_else(|| RepositoryError::InvalidPath(path.to_string()))?;
        entry.layer = target_layer;
        entry.updated_at = chrono::Utc::now().timestamp();
        self.store_governance_track_with_verb(ctx, entry, message, "promote")
            .await
    }

    pub async fn get_by_path(
        &self,
        ctx: mk_core::types::TenantContext,
        path: &str,
    ) -> Result<Option<KnowledgeEntry>, RepositoryError> {
        for layer in [
            KnowledgeLayer::Tenant,
            KnowledgeLayer::Org,
            KnowledgeLayer::Team,
            KnowledgeLayer::Project,
        ] {
            if let Some(entry) = self.get(ctx.clone(), layer, path).await? {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    fn resolve_meta_path(
        &self,
        ctx: &mk_core::types::TenantContext,
        kind: &str,
        id: &str,
    ) -> std::path::PathBuf {
        self.root_path
            .join(ctx.tenant_id.as_str())
            .join("_meta")
            .join(kind)
            .join(format!("{id}.json"))
    }

    async fn store_promotion_request_inner(
        &self,
        ctx: mk_core::types::TenantContext,
        request: mk_core::types::PromotionRequest,
    ) -> Result<mk_core::types::PromotionRequest, RepositoryError> {
        let path = self.resolve_meta_path(&ctx, "promotions", &request.id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, serde_json::to_string(&request)?).await?;
        self.commit(&format!("[promotion] upsert {}", request.id))?;
        Ok(request)
    }

    async fn get_promotion_request_inner(
        &self,
        ctx: &mk_core::types::TenantContext,
        id: &str,
    ) -> Result<Option<mk_core::types::PromotionRequest>, RepositoryError> {
        let path = self.resolve_meta_path(ctx, "promotions", id);
        if !path.exists() {
            return Ok(None);
        }
        let content = tokio::fs::read_to_string(&path).await?;
        Ok(Some(serde_json::from_str(&content)?))
    }

    async fn list_promotion_requests_inner(
        &self,
        ctx: &mk_core::types::TenantContext,
        status: Option<mk_core::types::PromotionRequestStatus>,
    ) -> Result<Vec<mk_core::types::PromotionRequest>, RepositoryError> {
        let dir = self
            .root_path
            .join(ctx.tenant_id.as_str())
            .join("_meta")
            .join("promotions");
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut results = Vec::new();
        for entry in WalkDir::new(&dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let content = tokio::fs::read_to_string(entry.path()).await?;
            if let Ok(req) = serde_json::from_str::<mk_core::types::PromotionRequest>(&content)
                && status.as_ref().is_none_or(|s| &req.status == s)
            {
                results.push(req);
            }
        }
        Ok(results)
    }

    async fn store_relation_inner(
        &self,
        ctx: mk_core::types::TenantContext,
        relation: mk_core::types::KnowledgeRelation,
    ) -> Result<mk_core::types::KnowledgeRelation, RepositoryError> {
        let path = self.resolve_meta_path(&ctx, "relations", &relation.id);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, serde_json::to_string(&relation)?).await?;
        self.commit(&format!("[relation] upsert {}", relation.id))?;
        Ok(relation)
    }

    async fn get_relations_for_item_inner(
        &self,
        ctx: &mk_core::types::TenantContext,
        item_id: &str,
    ) -> Result<Vec<mk_core::types::KnowledgeRelation>, RepositoryError> {
        let dir = self
            .root_path
            .join(ctx.tenant_id.as_str())
            .join("_meta")
            .join("relations");
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut results = Vec::new();
        for entry in WalkDir::new(&dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let content = tokio::fs::read_to_string(entry.path()).await?;
            if let Ok(rel) = serde_json::from_str::<mk_core::types::KnowledgeRelation>(&content)
                && (rel.source_id == item_id || rel.target_id == item_id)
            {
                results.push(rel);
            }
        }
        Ok(results)
    }

    /// Copies source entry to target layer WITHOUT modifying or deleting the source.
    #[allow(dead_code)]
    async fn promote_non_destructive(
        &self,
        ctx: mk_core::types::TenantContext,
        source_layer: KnowledgeLayer,
        target_layer: KnowledgeLayer,
        path: &str,
        message: &str,
    ) -> Result<String, RepositoryError> {
        let mut entry = self
            .get_inner(ctx.clone(), source_layer, path)
            .await?
            .ok_or_else(|| RepositoryError::InvalidPath(path.to_string()))?;
        entry.layer = target_layer;
        entry.updated_at = chrono::Utc::now().timestamp();
        if self.remote_url.is_some() {
            self.store_governance_track_with_verb(ctx, entry, message, "promote")
                .await
        } else {
            self.store_inner(ctx, entry, message).await
        }
    }
}

#[async_trait]
impl KnowledgeRepository for GitRepository {
    type Error = RepositoryError;

    async fn get_head_commit(
        &self,
        _ctx: mk_core::types::TenantContext,
    ) -> Result<Option<String>, Self::Error> {
        self.get_head_commit_sync()
    }

    async fn get_affected_items(
        &self,
        _ctx: mk_core::types::TenantContext,
        since_commit: &str,
    ) -> Result<Vec<(KnowledgeLayer, String)>, Self::Error> {
        let repo = Repository::open(&self.root_path)?;
        let from_obj = repo.revparse_single(since_commit)?;
        let from_commit = from_obj.peel_to_commit()?;
        let from_tree = from_commit.tree()?;

        let head = repo.head()?.peel_to_commit()?;
        let head_tree = head.tree()?;

        let diff = repo.diff_tree_to_tree(Some(&from_tree), Some(&head_tree), None)?;
        let mut affected = Vec::new();

        diff.foreach(
            &mut |delta, _| {
                if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
                    let parts: Vec<&str> = path.split('/').collect();
                    if parts.len() >= 2 {
                        let layer = match parts[0] {
                            "tenant" => KnowledgeLayer::Tenant,
                            "org" => KnowledgeLayer::Org,
                            "team" => KnowledgeLayer::Team,
                            "project" => KnowledgeLayer::Project,
                            _ => return true,
                        };
                        let inner_path = parts[1..].join("/");
                        affected.push((layer, inner_path));
                    }
                }
                true
            },
            None,
            None,
            None,
        )?;

        Ok(affected)
    }

    async fn get(
        &self,
        ctx: mk_core::types::TenantContext,
        layer: KnowledgeLayer,
        path: &str,
    ) -> Result<Option<KnowledgeEntry>, Self::Error> {
        if self.remote_url.is_some() {
            let _lock = self.rw_lock.read().await;
            self.pull_from_remote()?;
            self.get_inner(ctx, layer, path).await
        } else {
            self.get_inner(ctx, layer, path).await
        }
    }

    async fn store(
        &self,
        ctx: mk_core::types::TenantContext,
        entry: KnowledgeEntry,
        message: &str,
    ) -> Result<String, Self::Error> {
        if self.remote_url.is_some() {
            let _lock = self.rw_lock.write().await;
            if requires_governance(entry.layer, entry.status, &WriteOperation::Create) {
                self.store_governance_track(ctx, entry, message).await
            } else {
                let sha = self.store_inner(ctx, entry, message).await?;
                self.push_to_remote()?;
                Ok(sha)
            }
        } else {
            self.store_inner(ctx, entry, message).await
        }
    }

    async fn list(
        &self,
        ctx: mk_core::types::TenantContext,
        layer: KnowledgeLayer,
        prefix: &str,
    ) -> Result<Vec<KnowledgeEntry>, Self::Error> {
        if self.remote_url.is_some() {
            let _lock = self.rw_lock.read().await;
            self.pull_from_remote()?;
            self.list_inner(ctx, layer, prefix).await
        } else {
            self.list_inner(ctx, layer, prefix).await
        }
    }

    async fn delete(
        &self,
        ctx: mk_core::types::TenantContext,
        layer: KnowledgeLayer,
        path: &str,
        message: &str,
    ) -> Result<String, Self::Error> {
        if self.remote_url.is_some() {
            let _lock = self.rw_lock.write().await;
            if requires_governance(layer, KnowledgeStatus::Draft, &WriteOperation::Delete) {
                self.delete_governance_track(ctx, layer, path, message)
                    .await
            } else {
                let sha = self.delete_inner(ctx, layer, path, message).await?;
                self.push_to_remote()?;
                Ok(sha)
            }
        } else {
            self.delete_inner(ctx, layer, path, message).await
        }
    }

    async fn search(
        &self,
        ctx: mk_core::types::TenantContext,
        query: &str,
        layers: Vec<KnowledgeLayer>,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>, Self::Error> {
        if self.remote_url.is_some() {
            let _lock = self.rw_lock.read().await;
            self.pull_from_remote()?;
            self.search_inner(ctx, query, layers, limit).await
        } else {
            self.search_inner(ctx, query, layers, limit).await
        }
    }

    async fn update_status(
        &self,
        ctx: mk_core::types::TenantContext,
        layer: KnowledgeLayer,
        path: &str,
        new_status: KnowledgeStatus,
        message: &str,
    ) -> Result<String, Self::Error> {
        if self.remote_url.is_some() {
            let _lock = self.rw_lock.write().await;
            if requires_governance(
                layer,
                new_status,
                &WriteOperation::StatusChange { to: new_status },
            ) {
                self.update_status_governance_track(ctx, layer, path, new_status, message)
                    .await
            } else {
                let sha = self
                    .update_status_local(ctx, layer, path, new_status, message)
                    .await?;
                self.push_to_remote()?;
                Ok(sha)
            }
        } else {
            self.update_status_local(ctx, layer, path, new_status, message)
                .await
        }
    }

    async fn promote(
        &self,
        ctx: mk_core::types::TenantContext,
        source_layer: KnowledgeLayer,
        target_layer: KnowledgeLayer,
        path: &str,
        message: &str,
    ) -> Result<String, Self::Error> {
        if self.remote_url.is_some() {
            let _lock = self.rw_lock.write().await;
            self.promote_governance_track(ctx, source_layer, target_layer, path, message)
                .await
        } else {
            self.promote_local(ctx, source_layer, target_layer, path, message)
                .await
        }
    }

    fn root_path(&self) -> Option<std::path::PathBuf> {
        Some(self.root_path.clone())
    }

    async fn store_promotion_request(
        &self,
        ctx: mk_core::types::TenantContext,
        request: mk_core::types::PromotionRequest,
    ) -> Result<mk_core::types::PromotionRequest, Self::Error> {
        if self.remote_url.is_some() {
            let _lock = self.rw_lock.write().await;
            let r = self.store_promotion_request_inner(ctx, request).await?;
            self.push_to_remote()?;
            Ok(r)
        } else {
            self.store_promotion_request_inner(ctx, request).await
        }
    }

    async fn get_promotion_request(
        &self,
        ctx: mk_core::types::TenantContext,
        id: &str,
    ) -> Result<Option<mk_core::types::PromotionRequest>, Self::Error> {
        if self.remote_url.is_some() {
            let _lock = self.rw_lock.read().await;
            self.pull_from_remote()?;
            self.get_promotion_request_inner(&ctx, id).await
        } else {
            self.get_promotion_request_inner(&ctx, id).await
        }
    }

    async fn update_promotion_request(
        &self,
        ctx: mk_core::types::TenantContext,
        request: mk_core::types::PromotionRequest,
    ) -> Result<mk_core::types::PromotionRequest, Self::Error> {
        // Same as store — upsert by id
        if self.remote_url.is_some() {
            let _lock = self.rw_lock.write().await;
            let r = self.store_promotion_request_inner(ctx, request).await?;
            self.push_to_remote()?;
            Ok(r)
        } else {
            self.store_promotion_request_inner(ctx, request).await
        }
    }

    async fn list_promotion_requests(
        &self,
        ctx: mk_core::types::TenantContext,
        status: Option<mk_core::types::PromotionRequestStatus>,
    ) -> Result<Vec<mk_core::types::PromotionRequest>, Self::Error> {
        if self.remote_url.is_some() {
            let _lock = self.rw_lock.read().await;
            self.pull_from_remote()?;
            self.list_promotion_requests_inner(&ctx, status).await
        } else {
            self.list_promotion_requests_inner(&ctx, status).await
        }
    }

    async fn store_relation(
        &self,
        ctx: mk_core::types::TenantContext,
        relation: mk_core::types::KnowledgeRelation,
    ) -> Result<mk_core::types::KnowledgeRelation, Self::Error> {
        if self.remote_url.is_some() {
            let _lock = self.rw_lock.write().await;
            let r = self.store_relation_inner(ctx, relation).await?;
            self.push_to_remote()?;
            Ok(r)
        } else {
            self.store_relation_inner(ctx, relation).await
        }
    }

    async fn get_relations_for_item(
        &self,
        ctx: mk_core::types::TenantContext,
        item_id: &str,
    ) -> Result<Vec<mk_core::types::KnowledgeRelation>, Self::Error> {
        if self.remote_url.is_some() {
            let _lock = self.rw_lock.read().await;
            self.pull_from_remote()?;
            self.get_relations_for_item_inner(&ctx, item_id).await
        } else {
            self.get_relations_for_item_inner(&ctx, item_id).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_provider::{
        GitProviderError, MergeMethod, PullRequestInfo, PullRequestState, WebhookEvent,
    };
    use async_trait::async_trait;
    use tempfile::tempdir;

    struct TestGitProvider;

    #[async_trait]
    impl GitProvider for TestGitProvider {
        async fn create_branch(
            &self,
            _name: &str,
            _from_sha: &str,
        ) -> Result<(), GitProviderError> {
            Ok(())
        }

        async fn commit_to_branch(
            &self,
            _branch: &str,
            _path: &str,
            _content: &[u8],
            _message: &str,
        ) -> Result<String, GitProviderError> {
            Ok("mock-commit".to_string())
        }

        async fn create_pull_request(
            &self,
            _title: &str,
            _body: &str,
            _head: &str,
            _base: &str,
        ) -> Result<PullRequestInfo, GitProviderError> {
            Ok(PullRequestInfo {
                number: 42,
                title: "mock".to_string(),
                body: None,
                head_branch: "governance/mock".to_string(),
                base_branch: "main".to_string(),
                state: PullRequestState::Open,
                html_url: "https://example.invalid/pr/42".to_string(),
                merged: false,
                merge_commit_sha: None,
            })
        }

        async fn merge_pull_request(
            &self,
            _pr_number: u64,
            _merge_method: MergeMethod,
        ) -> Result<String, GitProviderError> {
            Ok("mock-merge".to_string())
        }

        async fn list_open_prs(
            &self,
            _head_prefix: Option<&str>,
        ) -> Result<Vec<PullRequestInfo>, GitProviderError> {
            Ok(vec![])
        }

        async fn parse_webhook(
            &self,
            _event_type: &str,
            _signature: Option<&str>,
            _body: &[u8],
        ) -> Result<WebhookEvent, GitProviderError> {
            Ok(WebhookEvent::Unknown {
                event_type: "mock".to_string(),
            })
        }

        async fn get_default_branch_sha(&self) -> Result<String, GitProviderError> {
            Ok("base-sha".to_string())
        }

        async fn get_installation_token(&self) -> Result<String, GitProviderError> {
            Ok("mock-token".to_string())
        }
    }

    #[tokio::test]
    async fn test_git_repository_lifecycle() -> Result<(), anyhow::Error> {
        let dir = tempdir()?;
        let repo = GitRepository::new(dir.path())?;
        let tenant_id = mk_core::types::TenantId::new("c1".into()).unwrap();
        let user_id = mk_core::types::UserId::new("u1".into()).unwrap();
        let ctx = mk_core::types::TenantContext::new(tenant_id, user_id);

        let entry = KnowledgeEntry {
            path: "test.md".to_string(),
            content: "hello world".to_string(),
            layer: KnowledgeLayer::Project,
            kind: KnowledgeType::Spec,
            status: mk_core::types::KnowledgeStatus::Draft,
            summaries: std::collections::HashMap::new(),
            metadata: std::collections::HashMap::new(),
            commit_hash: None,
            author: None,
            updated_at: chrono::Utc::now().timestamp(),
        };

        repo.store(ctx.clone(), entry.clone(), "initial commit")
            .await?;

        let retrieved = repo
            .get(ctx.clone(), KnowledgeLayer::Project, "test.md")
            .await?;
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.content, "hello world");
        assert_eq!(retrieved.status, mk_core::types::KnowledgeStatus::Draft);

        let list = repo.list(ctx.clone(), KnowledgeLayer::Project, "").await?;
        assert_eq!(list.len(), 1);

        repo.delete(
            ctx.clone(),
            KnowledgeLayer::Project,
            "test.md",
            "delete file",
        )
        .await?;
        let after_delete = repo.get(ctx, KnowledgeLayer::Project, "test.md").await?;
        assert!(after_delete.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn test_git_repository_isolation() -> Result<(), anyhow::Error> {
        let dir = tempdir()?;
        let repo = GitRepository::new(dir.path())?;

        let tenant_a = mk_core::types::TenantId::new("tenant_a".into()).unwrap();
        let user_a = mk_core::types::UserId::new("user_a".into()).unwrap();
        let ctx_a = mk_core::types::TenantContext::new(tenant_a, user_a);

        let tenant_b = mk_core::types::TenantId::new("tenant_b".into()).unwrap();
        let user_b = mk_core::types::UserId::new("user_b".into()).unwrap();
        let ctx_b = mk_core::types::TenantContext::new(tenant_b, user_b);

        let entry = KnowledgeEntry {
            path: "secret.md".to_string(),
            content: "tenant a secret".to_string(),
            layer: KnowledgeLayer::Project,
            kind: KnowledgeType::Spec,
            status: mk_core::types::KnowledgeStatus::Accepted,
            summaries: std::collections::HashMap::new(),
            metadata: std::collections::HashMap::new(),
            commit_hash: None,
            author: None,
            updated_at: chrono::Utc::now().timestamp(),
        };

        repo.store(ctx_a.clone(), entry, "tenant a commit").await?;

        let retrieved_b = repo
            .get(ctx_b.clone(), KnowledgeLayer::Project, "secret.md")
            .await?;
        assert!(
            retrieved_b.is_none(),
            "Tenant B should not see Tenant A data"
        );

        let retrieved_a = repo
            .get(ctx_a.clone(), KnowledgeLayer::Project, "secret.md")
            .await?;
        assert!(retrieved_a.is_some());
        assert_eq!(retrieved_a.unwrap().content, "tenant a secret");

        let list_b = repo.list(ctx_b, KnowledgeLayer::Project, "").await?;
        assert!(list_b.is_empty(), "Tenant B list should be empty");

        let list_a = repo.list(ctx_a, KnowledgeLayer::Project, "").await?;
        assert_eq!(list_a.len(), 1, "Tenant A should see its entry");

        Ok(())
    }
    #[tokio::test]
    async fn test_git_repository_path_traversal_protection() -> Result<(), anyhow::Error> {
        let dir = tempdir()?;
        let repo = GitRepository::new(dir.path())?;

        let tenant_a = mk_core::types::TenantId::new("tenant_a".into()).unwrap();
        let user_a = mk_core::types::UserId::new("user_a".into()).unwrap();
        let ctx_a = mk_core::types::TenantContext::new(tenant_a, user_a);

        // Attempt path traversal via filename
        let entry = KnowledgeEntry {
            path: "../../../etc/passwd".to_string(),
            content: "evil content".to_string(),
            layer: KnowledgeLayer::Project,
            kind: KnowledgeType::Spec,
            status: mk_core::types::KnowledgeStatus::Accepted,
            summaries: std::collections::HashMap::new(),
            metadata: std::collections::HashMap::new(),
            commit_hash: None,
            author: None,
            updated_at: chrono::Utc::now().timestamp(),
        };

        repo.store(ctx_a.clone(), entry, "malicious commit").await?;

        let full_path = repo.resolve_path(&ctx_a, KnowledgeLayer::Project, "../../../etc/passwd");

        // Ensure the resolved path is still within the tenant directory
        let tenant_dir = repo.root_path.join("tenant_a");
        assert!(
            full_path.starts_with(&tenant_dir),
            "Path should be constrained to tenant directory. Got: {:?}",
            full_path
        );

        Ok(())
    }

    #[test]
    fn test_new_with_remote_none_is_backward_compatible() -> Result<(), anyhow::Error> {
        let dir = tempdir()?;
        let repo = GitRepository::new_with_remote(dir.path(), None)?;
        assert!(repo.root_path().join(".git").exists());
        Ok(())
    }

    #[test]
    fn test_remote_config_construction() {
        let provider: Arc<dyn GitProvider> = Arc::new(TestGitProvider);
        let cfg = RemoteConfig {
            url: "https://github.com/org/repo.git".to_string(),
            branch: "main".to_string(),
            git_provider: Some(provider),
        };

        assert_eq!(cfg.url, "https://github.com/org/repo.git");
        assert_eq!(cfg.branch, "main");
        assert!(cfg.git_provider.is_some());
    }

    #[test]
    fn test_remote_callbacks_constructs() -> Result<(), anyhow::Error> {
        let dir = tempdir()?;
        let repo = GitRepository {
            root_path: dir.path().to_path_buf(),
            remote_url: Some("https://github.com/org/repo.git".to_string()),
            branch: "main".to_string(),
            git_provider: None,
            rw_lock: Arc::new(RwLock::new(())),
        };

        let callbacks = repo.remote_callbacks();
        let mut fetch_options = git2::FetchOptions::new();
        fetch_options.remote_callbacks(callbacks);
        Ok(())
    }

    #[test]
    fn test_pull_from_remote_without_remote_config_is_noop() -> Result<(), anyhow::Error> {
        let dir = tempdir()?;
        let repo = GitRepository::new(dir.path())?;
        repo.pull_from_remote()?;
        Ok(())
    }

    #[test]
    fn test_routing_project_draft_create_is_fast_track() {
        assert!(!requires_governance(
            KnowledgeLayer::Project,
            KnowledgeStatus::Draft,
            &WriteOperation::Create,
        ));
    }

    #[test]
    fn test_routing_team_draft_create_is_governance_track() {
        assert!(requires_governance(
            KnowledgeLayer::Team,
            KnowledgeStatus::Draft,
            &WriteOperation::Create,
        ));
    }

    #[test]
    fn test_routing_project_accepted_create_is_governance_track() {
        assert!(requires_governance(
            KnowledgeLayer::Project,
            KnowledgeStatus::Accepted,
            &WriteOperation::Create,
        ));
    }

    #[test]
    fn test_routing_status_change_to_accepted_is_governance_track() {
        assert!(requires_governance(
            KnowledgeLayer::Project,
            KnowledgeStatus::Draft,
            &WriteOperation::StatusChange {
                to: KnowledgeStatus::Accepted,
            },
        ));
    }

    #[test]
    fn test_routing_promote_always_governance_track() {
        assert!(requires_governance(
            KnowledgeLayer::Project,
            KnowledgeStatus::Draft,
            &WriteOperation::Promote {
                target_layer: KnowledgeLayer::Team,
            },
        ));
    }

    #[test]
    fn test_routing_delete_project_draft_is_fast_track() {
        assert!(!requires_governance(
            KnowledgeLayer::Project,
            KnowledgeStatus::Draft,
            &WriteOperation::Delete,
        ));
    }

    #[test]
    fn test_routing_delete_team_is_governance_track() {
        assert!(requires_governance(
            KnowledgeLayer::Team,
            KnowledgeStatus::Draft,
            &WriteOperation::Delete,
        ));
    }

    #[tokio::test]
    async fn test_no_remote_configured_always_local_mode() -> Result<(), anyhow::Error> {
        let dir = tempdir()?;
        let repo = GitRepository::new(dir.path())?;

        let tenant_id = mk_core::types::TenantId::new("c1".into()).unwrap();
        let user_id = mk_core::types::UserId::new("u1".into()).unwrap();
        let ctx = mk_core::types::TenantContext::new(tenant_id, user_id);

        let entry = KnowledgeEntry {
            path: "governed.md".to_string(),
            content: "team content".to_string(),
            layer: KnowledgeLayer::Team,
            kind: KnowledgeType::Spec,
            status: KnowledgeStatus::Accepted,
            summaries: std::collections::HashMap::new(),
            metadata: std::collections::HashMap::new(),
            commit_hash: None,
            author: None,
            updated_at: chrono::Utc::now().timestamp(),
        };

        let sha = repo.store(ctx.clone(), entry, "local write").await?;
        assert!(!sha.is_empty());

        let loaded = repo
            .get(ctx, KnowledgeLayer::Team, "governed.md")
            .await?
            .expect("entry should exist");
        assert_eq!(loaded.content, "team content");
        Ok(())
    }
}

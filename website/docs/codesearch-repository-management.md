# Code Search Repository Management: Local, Remote, and Incremental Indexing

## Problem Statement

The current Code Search integration only supports **static local directories**. Real-world use cases require:

1. **Local Development Repos** - Developer's active workspace with real-time changes
2. **Remote Team Repos** - Shared repositories with multiple branches
3. **Incremental Indexing** - Only re-index changes, not full codebase
4. **Hybrid Approach** - Local clones of remote repos with synchronization

## Solution Architecture

### Repository Types

#### 1. Local Repositories
**Use Case**: Active development on developer's machine

```yaml
repositories:
  - name: my-local-project
    type: local
    path: /workspace/my-project
    indexing:
      mode: watch  # Real-time file watching
      on_save: true
      debounce_ms: 500
```

**Characteristics**:
- Real-time file system watching (fsnotify/inotify)
- Immediate index updates on file changes
- No git required (can work with uncommitted changes)
- Lowest latency for search/trace

#### 2. Remote Repositories
**Use Case**: Team repositories, open-source projects, dependencies

```yaml
repositories:
  - name: tenant-backend
    type: remote
    url: https://github.com/example/backend.git
    branches:
      - main
      - develop
      - "feature/*"  # Wildcard support
    indexing:
      mode: poll
      interval: 5m  # Check for updates every 5 minutes
      on_push: webhook  # Optional: GitHub webhook trigger
    credentials:
      type: token
      secret: github-token
```

**Characteristics**:
- Automatic cloning on first use
- Periodic polling for updates (configurable)
- Multi-branch support with branch switching
- Git credentials management (SSH keys, tokens, OAuth)
- Optional webhook integration for instant updates

#### 3. Hybrid Repositories
**Use Case**: Local clone of remote repo with bidirectional sync

```yaml
repositories:
  - name: aeterna-dev
    type: hybrid
    url: https://github.com/kikokikok/aeterna.git
    local_path: /workspace/aeterna
    branch: main
    sync:
      mode: bidirectional
      auto_pull: true
      pull_interval: 10m
      watch_local: true  # Also watch local changes
```

**Characteristics**:
- Best of both worlds: local speed + remote updates
- Automatic git pull to stay synchronized
- Can switch branches locally
- Detects local uncommitted changes
- Smart conflict detection

### Incremental Indexing

#### Problem
Full re-indexing is expensive:
- Large codebases: 10,000+ files
- Embedding generation: 100-500ms per file
- Total time: 15-60 minutes for full index
- Resource intensive (CPU, memory, API costs)

#### Solution: Git-Based Change Detection

```rust
pub struct IncrementalIndexer {
    repo_path: PathBuf,
    last_indexed_commit: String,
    index_metadata: IndexMetadata,
}

impl IncrementalIndexer {
    /// Detect files changed since last index
    pub fn get_changed_files(&self) -> Result<Vec<ChangedFile>> {
        // git diff --name-status <last_commit>..HEAD
        let output = Command::new("git")
            .args(&["diff", "--name-status", 
                    &format!("{}..HEAD", self.last_indexed_commit)])
            .current_dir(&self.repo_path)
            .output()?;
        
        // Parse: A (added), M (modified), D (deleted), R (renamed)
        self.parse_diff_output(&output.stdout)
    }
    
    /// Perform incremental index update
    pub async fn update_index(&mut self) -> Result<IndexStats> {
        let changes = self.get_changed_files()?;
        
        let mut stats = IndexStats::default();
        
        for change in changes {
            match change.status {
                ChangeStatus::Added | ChangeStatus::Modified => {
                    // Generate embeddings for new/modified file
                    self.index_file(&change.path).await?;
                    stats.files_indexed += 1;
                }
                ChangeStatus::Deleted => {
                    // Remove from index
                    self.remove_from_index(&change.path).await?;
                    stats.files_removed += 1;
                }
                ChangeStatus::Renamed => {
                    // Update path in index
                    self.rename_in_index(&change.old_path, &change.new_path).await?;
                    stats.files_renamed += 1;
                }
            }
        }
        
        // Update metadata
        self.last_indexed_commit = self.get_current_commit()?;
        self.save_metadata()?;
        
        Ok(stats)
    }
}
```

**Benefits**:
- 10-100x faster than full re-index
- Proportional to change size (1 file = 1 file indexed)
- Lower API costs (only new/modified files)
- Can run more frequently

#### Smart Strategies

1. **File System Watch** (Local Repos)
   ```rust
   // Use notify crate for file system events
   let watcher = notify::watcher(tx, Duration::from_millis(500))?;
   watcher.watch(&repo_path, RecursiveMode::Recursive)?;
   
   // On file change
   async fn on_file_changed(path: &Path) {
       // Debounce to avoid re-indexing on every keystroke
       tokio::time::sleep(Duration::from_millis(500)).await;
       indexer.index_file(path).await?;
   }
   ```

2. **Git Hook Integration** (Hybrid/Local)
   ```bash
   # .git/hooks/post-commit
   #!/bin/bash
   aeterna codesearch index --incremental --async
   ```

3. **Webhook Triggers** (Remote Repos)
   ```yaml
   # GitHub webhook on push
   POST /api/v1/codesearch/webhook/github
   {
     "repository": "example/backend",
     "ref": "refs/heads/main",
     "commits": [...]
   }
   ```

### Repository Lifecycle Management

#### State Machine

```
┌──────────┐
│ INITIAL  │
└────┬─────┘
     │ add remote
     ▼
┌──────────┐
│ CLONING  │  ← git clone in progress
└────┬─────┘
     │ clone complete
     ▼
┌──────────┐
│ INDEXING │  ← initial full index
└────┬─────┘
     │ index complete
     ▼
┌──────────┐      ┌──────────┐
│  READY   │ ────▶│ UPDATING │  ← incremental update
└────┬─────┘      └────┬─────┘
     │                  │
     │◀─────────────────┘
     │
     ▼
┌──────────┐
│  ERROR   │  ← failed clone/index
└──────────┘
```

#### Database Schema

```sql
-- Repository metadata
CREATE TABLE codesearch_repositories (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id),
    name VARCHAR(255) NOT NULL,
    type VARCHAR(20) NOT NULL,  -- 'local', 'remote', 'hybrid'
    
    -- Remote repo fields
    remote_url TEXT,
    current_branch VARCHAR(255),
    tracked_branches TEXT[],
    
    -- Local repo fields
    local_path TEXT NOT NULL,
    
    -- Indexing state
    status VARCHAR(20) NOT NULL,  -- 'cloning', 'indexing', 'ready', 'error'
    last_indexed_commit VARCHAR(40),
    last_indexed_at TIMESTAMPTZ,
    last_updated_at TIMESTAMPTZ,
    
    -- Configuration
    config JSONB,  -- Type-specific config
    
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    
    UNIQUE(tenant_id, name)
);

-- Index metadata for incremental updates
CREATE TABLE codesearch_index_metadata (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    repository_id UUID NOT NULL REFERENCES codesearch_repositories(id),
    
    -- Commit tracking
    commit_sha VARCHAR(40) NOT NULL,
    parent_commit_sha VARCHAR(40),
    
    -- Statistics
    files_indexed INTEGER NOT NULL,
    files_removed INTEGER,
    files_renamed INTEGER,
    embeddings_generated INTEGER,
    
    -- Performance metrics
    indexing_duration_ms INTEGER,
    embedding_api_calls INTEGER,
    
    indexed_at TIMESTAMPTZ DEFAULT NOW()
);

-- Enable RLS for multi-tenancy
ALTER TABLE codesearch_repositories ENABLE ROW LEVEL SECURITY;
ALTER TABLE codesearch_index_metadata ENABLE ROW LEVEL SECURITY;

CREATE POLICY codesearch_repositories_tenant_isolation 
ON codesearch_repositories 
FOR ALL 
TO authenticated 
USING (tenant_id = current_setting('app.tenant_id', true)::UUID);

CREATE POLICY codesearch_index_metadata_tenant_isolation 
ON codesearch_index_metadata 
FOR ALL 
TO authenticated 
USING (
    repository_id IN (
        SELECT id FROM codesearch_repositories 
        WHERE tenant_id = current_setting('app.tenant_id', true)::UUID
    )
);
```

### Performance Considerations

#### Indexing Performance

| Strategy | Files Changed | Time (Approx) | API Calls |
|----------|---------------|---------------|-----------|
| Full Index | 10,000 | 30-60 min | 10,000 |
| Incremental (10 files) | 10 | 5-10 sec | 10 |
| Incremental (100 files) | 100 | 30-60 sec | 100 |
| Watch Mode (1 file) | 1 | <1 sec | 1 |

**Optimization Strategies**:
1. **Batch Processing**: Group file changes into batches
2. **Parallel Embedding**: Generate embeddings concurrently
3. **Caching**: Cache embeddings for unchanged files
4. **Smart Filtering**: Skip generated files, vendor directories
5. **Rate Limiting**: Respect embedding API rate limits

#### Storage Performance

| Backend | Index Size (10k files) | Query Latency | Best For |
|---------|------------------------|---------------|----------|
| Qdrant | 500 MB | 10-50ms | Production, multi-tenant |
| GOB (file-based) | 400 MB | 50-200ms | Dev/test, single-tenant |

### Multi-Branch Strategy

#### Use Cases

1. **Feature Branch Development**
   - Index main branch (stable reference)
   - Index feature branch (active development)
   - Switch between branches for context

2. **Code Review**
   - Index PR source branch
   - Index PR target branch
   - Compare call graphs, find breaking changes

3. **Multi-Version Support**
   - Index v1.x branch (maintenance)
   - Index v2.x branch (current)
   - Index v3.x branch (development)

#### Implementation

```rust
pub struct BranchManager {
    repo_path: PathBuf,
    current_branch: String,
    indexed_branches: HashMap<String, IndexHandle>,
}

impl BranchManager {
    /// Switch to a different branch and update index
    pub async fn checkout_branch(&mut self, branch: &str) -> Result<()> {
        // Check if branch already indexed
        if let Some(handle) = self.indexed_branches.get(branch) {
            // Switch active index
            self.activate_index(handle).await?;
            return Ok(());
        }
        
        // Git checkout
        Command::new("git")
            .args(&["checkout", branch])
            .current_dir(&self.repo_path)
            .output()?;
        
        // Check what changed from main branch
        let changed_files = self.diff_from_main(branch)?;
        
        if changed_files.len() < 100 {
            // Incremental index from main branch
            self.incremental_index_from_main(branch, changed_files).await?;
        } else {
            // Full index for this branch
            self.full_index(branch).await?;
        }
        
        self.current_branch = branch.to_string();
        Ok(())
    }
    
    /// Clean up old branch indexes to save space
    pub async fn prune_branches(&mut self, keep: Vec<String>) -> Result<()> {
        for (branch, handle) in &self.indexed_branches {
            if !keep.contains(branch) {
                self.remove_index(handle).await?;
            }
        }
        Ok(())
    }
}
```

### Webhook Integration

#### GitHub Webhooks

```rust
// API endpoint for GitHub webhooks
#[post("/api/v1/codesearch/webhook/github")]
async fn github_webhook(
    payload: web::Json<GitHubPayload>,
    secret: web::Data<String>,
) -> Result<HttpResponse> {
    // Verify webhook signature
    verify_github_signature(&payload, &secret)?;
    
    match payload.event {
        "push" => {
            // Extract repository and branch
            let repo_name = payload.repository.full_name;
            let branch = payload.ref_.trim_start_matches("refs/heads/");
            
            // Trigger incremental index
            tokio::spawn(async move {
                if let Some(repo) = find_repository(&repo_name).await {
                    repo.incremental_update().await?;
                }
            });
        }
        "pull_request" => {
            // Index PR branch on open/synchronize
            if ["opened", "synchronize"].contains(&payload.action.as_str()) {
                let branch = payload.pull_request.head.ref_;
                // Index PR branch
            }
        }
        _ => {}
    }
    
    Ok(HttpResponse::Ok().finish())
}
```

### CLI Commands

#### Extended Repository Management

```bash
# Add a remote repository
aeterna codesearch repo add \
  --name tenant-backend \
  --url https://github.com/example/backend.git \
  --branches main,develop \
  --poll-interval 5m

# List all tracked repositories
aeterna codesearch repo list
# Output:
# NAME              TYPE    STATUS    BRANCH    LAST INDEXED
# tenant-backend   remote  ready     main      2m ago
# my-local-project  local   ready     -         5s ago (watching)
# aeterna           hybrid  ready     main      10m ago

# Update a repository (pull + incremental index)
aeterna codesearch repo update tenant-backend
# Output:
# Pulling latest changes from main...
# Found 15 changed files
# Indexing incrementally...
# ✓ 15 files indexed in 8.3s

# Checkout a different branch
aeterna codesearch repo checkout tenant-backend develop
# Output:
# Checking out branch: develop
# Calculating delta from main: 42 files changed
# Indexing incrementally from main...
# ✓ Branch switched and indexed in 15.2s

# Remove a repository
aeterna codesearch repo remove tenant-backend
# Output:
# Removing repository and index...
# ✓ Repository removed

# Incremental index (manual trigger)
aeterna codesearch index --incremental
# Auto-detect changed files in all local/hybrid repos
# Output:
# Scanning repositories...
# Found 3 repositories with changes:
#   - my-local-project: 2 files
#   - aeterna: 5 files
# Indexing 7 files...
# ✓ Completed in 3.1s

# Force full re-index
aeterna codesearch index --full --repo tenant-backend
# Warning: This will re-index all 10,247 files
# Proceed? [y/N] y
# Indexing...
# ✓ Completed in 42m 15s
```

### Configuration Examples

#### Development Setup (Local Only)
```yaml
# values.yaml
codesearch:
  enabled: true
  
  repositories:
    - name: my-project
      type: local
      path: /workspace/my-project
      indexing:
        mode: watch
        on_save: true
  
  embedder:
    type: ollama
    model: nomic-embed-text
  
  store:
    type: gob  # Simple file-based for dev
```

#### Team Setup (Remote + Incremental)
```yaml
# values.yaml
codesearch:
  enabled: true
  
  repositories:
    # Main backend repo
    - name: backend
      type: remote
      url: https://github.com/example/backend.git
      branches: [main, develop]
      indexing:
        mode: poll
        interval: 5m
      credentials:
        secret: github-token
    
    # Frontend repo
    - name: frontend
      type: remote
      url: https://github.com/example/frontend.git
      branches: [main]
      indexing:
        mode: webhook  # GitHub webhook integration
      credentials:
        secret: github-token
    
    # Shared library
    - name: shared-lib
      type: remote
      url: https://github.com/example/shared-lib.git
      branches: ["v1.*", "v2.*"]
      indexing:
        mode: poll
        interval: 30m  # Less frequent
  
  embedder:
    type: openai
    model: text-embedding-3-small
  
  store:
    type: qdrant  # Production-grade
  
  # Webhook server for instant updates
  webhook:
    enabled: true
    port: 9091
    secret: webhook-secret
```

#### Hybrid Setup (Best of Both Worlds)
```yaml
# values.yaml
codesearch:
  enabled: true
  
  repositories:
    # Active development (local clone + remote sync)
    - name: my-feature
      type: hybrid
      url: https://github.com/example/backend.git
      local_path: /workspace/backend
      branch: feature/my-feature
      sync:
        auto_pull: true
        pull_interval: 10m
        watch_local: true
      credentials:
        secret: github-token
    
    # Reference repos (remote only)
    - name: main-backend
      type: remote
      url: https://github.com/example/backend.git
      branches: [main]
      indexing:
        mode: poll
        interval: 15m
  
  embedder:
    type: ollama
  
  store:
    type: qdrant
```

### Migration Path

#### From Standalone Code Search

**Before** (Manual Code Search):
```bash
# Manual setup
cd /workspace/project
codesearch init --embedder ollama --store gob

# Manual updates
git pull
codesearch init --force  # Full re-index :(
```

**After** (Aeterna + Code Search):
```bash
# One-time setup
aeterna codesearch repo add \
  --name project \
  --type hybrid \
  --url https://github.com/org/project.git \
  --local-path /workspace/project

# Automatic updates
# - Git hooks trigger incremental index
# - Or periodic polling
# - Or webhook from GitHub

# Manual update when needed
aeterna codesearch repo update project  # Incremental only!
```

**Benefits**:
- 10-100x faster updates (incremental vs full)
- Automatic synchronization
- Multi-repository support
- Branch management
- Tenant isolation

### Best Practices

1. **Choose the Right Repository Type**
   - Local: Active development, low latency needed
   - Remote: Team repos, read-only access
   - Hybrid: Feature branches, need local changes + remote sync

2. **Optimize Indexing Frequency**
   - Local watch: Real-time (500ms debounce)
   - Hybrid poll: 10-15 minutes
   - Remote poll: 5-30 minutes (depends on activity)
   - Webhook: Instant (preferred for remote)

3. **Manage Branch Indexes**
   - Index main/develop permanently
   - Index feature branches on-demand
   - Prune old feature branch indexes
   - Keep last 5-10 branch indexes

4. **Monitor Resource Usage**
   - Embedding API quota (OpenAI: 3M tokens/min)
   - Storage space (500MB per 10k files)
   - CPU usage during indexing
   - Set appropriate rate limits

5. **Security**
   - Store git credentials in Kubernetes secrets
   - Use deploy keys (read-only) for remote repos
   - Enable RLS for multi-tenancy
   - Audit repository access

### Troubleshooting

#### Issue: Incremental index missing changes
**Cause**: Git commit history not properly tracked
**Solution**:
```bash
# Check index metadata
aeterna codesearch repo status tenant-backend --verbose
# Last indexed commit: abc123
# Current commit: xyz789

# Force re-sync
aeterna codesearch index --full --repo tenant-backend
```

#### Issue: Webhook not triggering
**Cause**: Webhook secret mismatch or network issue
**Solution**:
```bash
# Test webhook locally
curl -X POST http://localhost:9091/api/v1/codesearch/webhook/github \
  -H "X-Hub-Signature-256: sha256=..." \
  -d @webhook-payload.json

# Check webhook logs
kubectl logs -n aeterna <pod> -c codesearch --tail=100
```

#### Issue: High embedding API costs
**Cause**: Full re-indexing too frequently
**Solution**:
```yaml
# Switch to incremental only
repositories:
  - name: backend
    indexing:
      mode: poll
      interval: 30m  # Increase interval
      strategy: incremental  # Never full re-index
```

### Future Enhancements

1. **Monorepo Support**
   - Index subdirectories as separate "projects"
   - Shared root, independent indexes
   - Workspace-aware searching

2. **Multi-Language Optimization**
   - Language-specific parsing
   - Cross-language call graphs
   - Language-aware chunking

3. **Distributed Indexing**
   - Shard large repos across workers
   - Parallel embedding generation
   - Index merging

4. **AI-Powered Index Optimization**
   - Learn frequently accessed files
   - Pre-index predicted files
   - Smart cache warming

## Conclusion

This repository management strategy provides:
- ✅ **Flexibility**: Local, remote, and hybrid support
- ✅ **Performance**: 10-100x faster with incremental indexing
- ✅ **Scale**: Multi-repository, multi-branch, multi-tenant
- ✅ **Cost**: Proportional API costs (only index changes)
- ✅ **Developer Experience**: Automatic updates, minimal friction

**Implementation Priority**:
1. Repository manager and database schema (1 week)
2. Incremental indexing engine (1 week)
3. CLI commands for repo management (3 days)
4. Webhook integration (3 days)
5. Multi-branch support (3 days)
6. Documentation and testing (3 days)

**Total Estimated Effort**: 3-4 weeks

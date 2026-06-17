/// Backup manifest types describing the contents and metadata of an archive.
///
/// The manifest is stored as `manifest.json` at the root of every backup
/// archive and contains entity counts, file checksums, and backend snapshot
/// identifiers so that a restore can verify integrity before touching any
/// live data.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current schema version for backup archives produced by this build.
pub const CURRENT_SCHEMA_VERSION: &str = "1.0.0";

/// Top-level manifest embedded in every backup archive.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BackupManifest {
    /// Semantic version of the archive schema (e.g. `"1.0.0"`).
    pub schema_version: String,
    /// ISO 8601 timestamp of when the backup was created.
    pub created_at: String,
    /// Hostname or unique identifier of the source Aeterna instance.
    pub source_instance: String,
    /// What scope of data this archive covers.
    pub scope: ExportScope,
    /// Whether this is an incremental (delta) backup.
    pub incremental: bool,
    /// For incremental backups, the Unix-epoch timestamp of the baseline.
    pub since_timestamp: Option<i64>,
    /// Counts of each entity type included in the archive.
    pub entity_counts: EntityCounts,
    /// Map of `filename -> SHA-256 hex digest` for every data file in the archive.
    pub file_checksums: HashMap<String, String>,
    /// Snapshot identifiers from the source backends at export time.
    pub backend_snapshots: BackendSnapshots,
    /// The embedding model used to produce any vectors in the archive.
    pub embedding_model: Option<String>,
}

/// The scope of data captured by a backup.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExportScope {
    /// The entire instance (all tenants, all layers).
    FullInstance,
    /// A single tenant.
    Tenant {
        /// The tenant identifier.
        tenant_id: String,
    },
    /// A single memory layer across all tenants.
    Layer {
        /// The layer name (e.g. `"episodic"`, `"semantic"`).
        layer: String,
    },
}

/// Counts of every exportable entity type.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct EntityCounts {
    /// Number of memory entries.
    pub memories: u64,
    /// Number of knowledge items.
    pub knowledge_items: u64,
    /// Number of knowledge-to-knowledge relations.
    pub knowledge_relations: u64,
    /// Number of graph nodes.
    pub graph_nodes: u64,
    /// Number of graph edges.
    pub graph_edges: u64,
    /// Number of Cedar/OPAL policies.
    pub policies: u64,
    /// Number of organizational units (tenants, orgs, teams, projects).
    pub org_units: u64,
    /// Number of role assignments.
    pub role_assignments: u64,
    /// Number of pending promotion requests.
    pub promotion_requests: u64,
    /// Number of governance audit events.
    pub governance_events: u64,
}

/// Point-in-time identifiers captured from each backend during export.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct BackendSnapshots {
    /// The PostgreSQL transaction start timestamp (ISO 8601).
    pub postgres_txn_start: Option<String>,
    /// The Qdrant snapshot identifier.
    pub qdrant_snapshot_id: Option<String>,
    /// The DuckDB read timestamp (ISO 8601).
    pub duckdb_read_at: Option<String>,
}

impl BackupManifest {
    /// Create a new manifest with the current schema version and sensible defaults.
    pub fn new(source_instance: String, scope: ExportScope) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            source_instance,
            scope,
            incremental: false,
            since_timestamp: None,
            entity_counts: EntityCounts::default(),
            file_checksums: HashMap::new(),
            backend_snapshots: BackendSnapshots::default(),
            embedding_model: None,
        }
    }

    /// Returns `true` if this archive's schema version is compatible with the
    /// running system (currently: exact match on major version).
    pub fn is_schema_compatible(&self) -> bool {
        // For now, check that the major version matches.
        let archive_major = self.schema_version.split('.').next().unwrap_or("0");
        let system_major = CURRENT_SCHEMA_VERSION.split('.').next().unwrap_or("0");
        archive_major == system_major
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> BackupManifest {
        BackupManifest {
            schema_version: "1.0.0".to_string(),
            created_at: "2026-04-11T12:00:00Z".to_string(),
            source_instance: "test-host".to_string(),
            scope: ExportScope::FullInstance,
            incremental: false,
            since_timestamp: None,
            entity_counts: EntityCounts {
                memories: 42,
                knowledge_items: 10,
                knowledge_relations: 5,
                graph_nodes: 3,
                graph_edges: 2,
                policies: 1,
                org_units: 4,
                role_assignments: 6,
                promotion_requests: 0,
                governance_events: 7,
            },
            file_checksums: HashMap::from([("memories.ndjson".to_string(), "abc123".to_string())]),
            backend_snapshots: BackendSnapshots {
                postgres_txn_start: Some("2026-04-11T12:00:00Z".to_string()),
                qdrant_snapshot_id: None,
                duckdb_read_at: None,
            },
            embedding_model: Some("text-embedding-3-small".to_string()),
        }
    }

    #[test]
    fn round_trip_json() {
        let manifest = sample_manifest();
        let json = serde_json::to_string_pretty(&manifest).expect("serialize");
        let back: BackupManifest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.schema_version, "1.0.0");
        assert_eq!(back.entity_counts.memories, 42);
        assert_eq!(back.source_instance, "test-host");
    }

    #[test]
    fn round_trip_tenant_scope() {
        let manifest = BackupManifest::new(
            "host1".into(),
            ExportScope::Tenant {
                tenant_id: "acme".into(),
            },
        );
        let json = serde_json::to_string(&manifest).expect("serialize");
        let back: BackupManifest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            back.scope,
            ExportScope::Tenant {
                tenant_id: "acme".into()
            }
        );
    }

    #[test]
    fn round_trip_layer_scope() {
        let manifest = BackupManifest::new(
            "host1".into(),
            ExportScope::Layer {
                layer: "episodic".into(),
            },
        );
        let json = serde_json::to_string(&manifest).expect("serialize");
        let back: BackupManifest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            back.scope,
            ExportScope::Layer {
                layer: "episodic".into()
            }
        );
    }

    #[test]
    fn schema_compatible_same_major() {
        let m = sample_manifest();
        assert!(m.is_schema_compatible());
    }

    #[test]
    fn schema_compatible_different_minor() {
        let mut m = sample_manifest();
        m.schema_version = "1.2.0".to_string();
        assert!(m.is_schema_compatible());
    }

    #[test]
    fn schema_incompatible_different_major() {
        let mut m = sample_manifest();
        m.schema_version = "2.0.0".to_string();
        assert!(!m.is_schema_compatible());
    }

    #[test]
    fn new_manifest_defaults() {
        let m = BackupManifest::new("node-1".into(), ExportScope::FullInstance);
        assert_eq!(m.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(!m.incremental);
        assert_eq!(m.entity_counts.memories, 0);
        assert!(m.file_checksums.is_empty());
    }

    #[test]
    fn entity_counts_default_zero() {
        let c = EntityCounts::default();
        assert_eq!(c.memories, 0);
        assert_eq!(c.knowledge_items, 0);
        assert_eq!(c.graph_nodes, 0);
        assert_eq!(c.policies, 0);
    }
}

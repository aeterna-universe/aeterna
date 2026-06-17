//! # Memory-Knowledge System Core
//!
//! Shared types, traits, and utilities for the Memory-Knowledge system.
//!
//! This crate provides:
//! - Type definitions for memory and knowledge systems
//! - Core traits for adapters and providers
//! - Error types with proper handling
//! - Validation utilities
//! - Operation hints for capability toggles
//!
//! # Best Practices
//!
//! - Follows Microsoft Pragmatic Rust Guidelines
//! - Uses Rust Edition 2024 (never back)
//! - Comprehensive error handling with `thiserror`
//! - M-CANONICAL-DOCS documentation format

pub mod env;
pub mod hints;
pub mod secret;
pub mod traits;
pub mod types;

// Re-export commonly used types for convenience
pub use env::Environment;
pub use hints::{HintPreset, HintsConfig, OperationHints};
pub use secret::{SecretBytes, SecretReference};
pub use types::{
    AccountRecord, AccountRef, BranchPolicy, ConstraintOperator, ConstraintSeverity,
    ConstraintTarget, CredentialKind, HierarchyPath, KnowledgeEntry, KnowledgeEntryWithRelations,
    KnowledgeLayer, KnowledgeQueryResult, KnowledgeRelation, KnowledgeRelationType,
    KnowledgeStatus, KnowledgeType, KnowledgeVariantRole, MemoryLayer, PromotionDecision,
    PromotionMode, PromotionRequest, PromotionRequestStatus, RecordSource, RepositoryKind,
    TenantConfigDocument, TenantConfigField, TenantConfigOwnership, TenantContext, TenantId,
    TenantRecord, TenantRepositoryBinding, TenantSecretEntry, TenantSecretReference, TenantStatus,
    UserId,
};

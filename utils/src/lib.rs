//! # Memory-Knowledge Utilities
//!
//! Common utility functions for hashing, validation, and UUID generation.
//!
//! # Best Practices
//!
//! - Uses SHA-2 for secure hashing
//! - Uses UUID v4 with serde support
//! - Validates inputs with comprehensive error messages

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Compute SHA-256 hash of content string
///
/// # Examples
///
/// ```
/// use utils::compute_content_hash;
///
/// let hash = compute_content_hash("hello world");
/// assert_eq!(hash.len(), 64);
/// ```
#[must_use]
pub fn compute_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

/// Compute hash of knowledge item for change detection
///
/// Hashes content, constraints, and status fields.
#[must_use]
pub fn compute_knowledge_hash(item: &serde_json::Value) -> String {
    let mut hasher = Sha256::new();

    // Extract fields for hashing
    if let Some(content_str) = item.get("content").and_then(|c| c.as_str()) {
        hasher.update(content_str.as_bytes());
    }

    if let Some(constraints) = item.get("constraints") {
        let constraints_json =
            serde_json::to_string(constraints).expect("Failed to serialize constraints");
        hasher.update(constraints_json.as_bytes());
    }

    if let Some(status_str) = item.get("status").and_then(|s| s.as_str()) {
        hasher.update(status_str.as_bytes());
    }

    hex::encode(hasher.finalize())
}

/// Generate UUID v4 string
#[must_use]
pub fn generate_uuid() -> String {
    Uuid::new_v4().to_string()
}

/// Validate memory layer string
#[must_use]
pub fn is_valid_layer(layer: &str) -> bool {
    matches!(
        layer,
        "agent" | "user" | "session" | "project" | "team" | "org" | "tenant"
    )
}

/// Validate knowledge type string
#[must_use]
pub fn is_valid_knowledge_type(ktype: &str) -> bool {
    matches!(ktype, "adr" | "policy" | "pattern" | "spec")
}

/// Validate knowledge layer string
#[must_use]
pub fn is_valid_knowledge_layer(layer: &str) -> bool {
    matches!(layer, "tenant" | "org" | "team" | "project")
}

/// Redact PII from content string
///
/// Currently redacts emails and simple phone numbers.
#[must_use]
pub fn redact_pii(content: &str) -> String {
    let email_re = regex::Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap();
    let result = email_re.replace_all(content, "[REDACTED_EMAIL]");

    let phone_re = regex::Regex::new(r"\d{3}-\d{3}-\d{4}").unwrap();
    phone_re
        .replace_all(&result, "[REDACTED_PHONE]")
        .to_string()
}

/// Get layer precedence value for memory layers
#[must_use]
pub fn get_layer_precedence(layer: &str) -> u8 {
    match layer {
        "agent" => 1,
        "user" => 2,
        "session" => 3,
        "project" => 4,
        "team" => 5,
        "org" => 6,
        "tenant" => 7,
        _ => 7, // Default to lowest precedence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_content_hash_consistency() {
        let content = "test content";
        let hash1 = compute_content_hash(content);
        let hash2 = compute_content_hash(content);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_generate_uuid_uniqueness() {
        let uuid1 = generate_uuid();
        let uuid2 = generate_uuid();
        assert_ne!(uuid1, uuid2);
    }

    #[test]
    fn test_layer_validation_valid() {
        assert!(is_valid_layer("agent"));
        assert!(is_valid_layer("user"));
        assert!(is_valid_layer("tenant"));
    }

    #[test]
    fn test_layer_validation_invalid() {
        assert!(!is_valid_layer("invalid"));
        assert!(!is_valid_layer("agent-user"));
    }

    #[test]
    fn test_redact_pii() {
        let content = "Contact alice@example.com at 123-456-7890.";
        let redacted = redact_pii(content);
        assert_eq!(redacted, "Contact [REDACTED_EMAIL] at [REDACTED_PHONE].");
    }

    #[test]
    fn test_compute_knowledge_hash() {
        let item = serde_json::json!({
            "content": "test content",
            "status": "accepted",
            "constraints": ["rule1"]
        });
        let hash = compute_knowledge_hash(&item);
        assert_eq!(hash.len(), 64);

        let item2 = serde_json::json!({
            "content": "test content",
            "status": "accepted",
            "constraints": ["rule1"]
        });
        assert_eq!(hash, compute_knowledge_hash(&item2));

        let item3 = serde_json::json!({
            "content": "different content",
            "status": "accepted",
            "constraints": ["rule1"]
        });
        assert_ne!(hash, compute_knowledge_hash(&item3));
    }

    #[test]
    fn test_is_valid_knowledge_type() {
        assert!(is_valid_knowledge_type("adr"));
        assert!(is_valid_knowledge_type("policy"));
        assert!(!is_valid_knowledge_type("unknown"));
    }

    #[test]
    fn test_is_valid_knowledge_layer() {
        assert!(is_valid_knowledge_layer("project"));
        assert!(is_valid_knowledge_layer("tenant"));
        assert!(!is_valid_knowledge_layer("user"));
    }

    #[test]
    fn test_get_layer_precedence() {
        assert_eq!(get_layer_precedence("agent"), 1);
        assert_eq!(get_layer_precedence("tenant"), 7);
        assert_eq!(get_layer_precedence("unknown"), 7);
    }
}

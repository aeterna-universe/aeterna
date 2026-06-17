use mk_core::types::{KnowledgeLayer, KnowledgeType, MemoryLayer, SummaryDepth};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePointer {
    pub source_type: KnowledgeType,
    pub source_id: String,
    pub content_hash: String,
    pub synced_at: i64,
    pub source_layer: KnowledgeLayer,
    pub is_orphaned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SummaryPointer {
    pub entry_id: String,
    pub layer: MemoryLayer,
    pub depth: SummaryDepth,
    pub content_hash: String,
    pub source_content_hash: String,
    pub token_count: u32,
    pub synced_at: i64,
    pub is_stale: bool,
    pub personalized: bool,
    pub personalization_context: Option<String>,
}

impl SummaryPointer {
    pub fn new(
        entry_id: String,
        layer: MemoryLayer,
        depth: SummaryDepth,
        content_hash: String,
        source_content_hash: String,
        token_count: u32,
    ) -> Self {
        Self {
            entry_id,
            layer,
            depth,
            content_hash,
            source_content_hash,
            token_count,
            synced_at: chrono::Utc::now().timestamp(),
            is_stale: false,
            personalized: false,
            personalization_context: None,
        }
    }

    pub fn mark_stale(&mut self) {
        self.is_stale = true;
    }

    pub fn needs_update(&self, current_source_hash: &str) -> bool {
        self.is_stale || self.source_content_hash != current_source_hash
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SummaryPointerState {
    pub pointers: HashMap<String, HashMap<SummaryDepth, SummaryPointer>>,
    pub last_sync_at: Option<i64>,
    pub total_summaries: u64,
    pub stale_count: u64,
}

impl SummaryPointerState {
    pub fn get_pointer(&self, entry_id: &str, depth: SummaryDepth) -> Option<&SummaryPointer> {
        self.pointers.get(entry_id).and_then(|m| m.get(&depth))
    }

    pub fn set_pointer(&mut self, pointer: SummaryPointer) {
        let entry = self.pointers.entry(pointer.entry_id.clone()).or_default();
        if !entry.contains_key(&pointer.depth) {
            self.total_summaries += 1;
        }
        entry.insert(pointer.depth, pointer);
    }

    pub fn remove_pointer(
        &mut self,
        entry_id: &str,
        depth: SummaryDepth,
    ) -> Option<SummaryPointer> {
        if let Some(entry) = self.pointers.get_mut(entry_id)
            && let Some(ptr) = entry.remove(&depth)
        {
            self.total_summaries = self.total_summaries.saturating_sub(1);
            if ptr.is_stale {
                self.stale_count = self.stale_count.saturating_sub(1);
            }
            if entry.is_empty() {
                self.pointers.remove(entry_id);
            }
            return Some(ptr);
        }
        None
    }

    pub fn remove_all_for_entry(&mut self, entry_id: &str) -> Vec<SummaryPointer> {
        if let Some(entry) = self.pointers.remove(entry_id) {
            let removed: Vec<_> = entry.into_values().collect();
            for ptr in &removed {
                self.total_summaries = self.total_summaries.saturating_sub(1);
                if ptr.is_stale {
                    self.stale_count = self.stale_count.saturating_sub(1);
                }
            }
            removed
        } else {
            Vec::new()
        }
    }

    pub fn mark_stale(&mut self, entry_id: &str, depth: SummaryDepth) -> bool {
        if let Some(ptr) = self
            .pointers
            .get_mut(entry_id)
            .and_then(|m| m.get_mut(&depth))
        {
            if !ptr.is_stale {
                ptr.is_stale = true;
                self.stale_count += 1;
            }
            true
        } else {
            false
        }
    }

    pub fn mark_all_stale_for_entry(&mut self, entry_id: &str) -> u32 {
        let mut count = 0;
        if let Some(entry) = self.pointers.get_mut(entry_id) {
            for ptr in entry.values_mut() {
                if !ptr.is_stale {
                    ptr.is_stale = true;
                    self.stale_count += 1;
                    count += 1;
                }
            }
        }
        count
    }

    /// Mark stale only the pointers whose `layer` matches the given layer.
    pub fn mark_stale_for_entry_and_layer(
        &mut self,
        entry_id: &str,
        layer: mk_core::types::MemoryLayer,
    ) -> u32 {
        let mut count = 0;
        if let Some(entry) = self.pointers.get_mut(entry_id) {
            for ptr in entry.values_mut() {
                if ptr.layer == layer && !ptr.is_stale {
                    ptr.is_stale = true;
                    self.stale_count += 1;
                    count += 1;
                }
            }
        }
        count
    }

    pub fn get_stale_pointers(&self) -> Vec<&SummaryPointer> {
        self.pointers
            .values()
            .flat_map(|m| m.values())
            .filter(|p| p.is_stale)
            .collect()
    }

    pub fn get_pointers_for_layer(&self, layer: MemoryLayer) -> Vec<&SummaryPointer> {
        self.pointers
            .values()
            .flat_map(|m| m.values())
            .filter(|p| p.layer == layer)
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HindsightPointer {
    pub error_signature_id: String,
    pub memory_entry_id: Option<String>,
    pub resolution_ids: Vec<String>,
    pub note_id: Option<String>,
    pub source_layer: MemoryLayer,
    pub synced_at: i64,
    pub success_rate: f32,
    pub application_count: u32,
}

impl HindsightPointer {
    pub fn new(error_signature_id: String, source_layer: MemoryLayer) -> Self {
        Self {
            error_signature_id,
            memory_entry_id: None,
            resolution_ids: Vec::new(),
            note_id: None,
            source_layer,
            synced_at: chrono::Utc::now().timestamp(),
            success_rate: 0.0,
            application_count: 0,
        }
    }

    pub fn add_resolution(&mut self, resolution_id: String) {
        if !self.resolution_ids.contains(&resolution_id) {
            self.resolution_ids.push(resolution_id);
        }
    }

    pub fn update_success_rate(&mut self, success: bool) {
        self.application_count += 1;
        let current_successes = (self.success_rate * (self.application_count - 1) as f32).round();
        let new_successes = if success {
            current_successes + 1.0
        } else {
            current_successes
        };
        self.success_rate = new_successes / self.application_count as f32;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct HindsightPointerState {
    pub pointers: HashMap<String, HindsightPointer>,
    pub last_sync_at: Option<i64>,
    pub total_patterns: u64,
    pub total_resolutions: u64,
}

impl HindsightPointerState {
    pub fn get_pointer(&self, error_signature_id: &str) -> Option<&HindsightPointer> {
        self.pointers.get(error_signature_id)
    }

    pub fn set_pointer(&mut self, pointer: HindsightPointer) {
        let is_new = !self.pointers.contains_key(&pointer.error_signature_id);
        let new_resolutions = pointer.resolution_ids.len() as u64;

        if let Some(existing) = self.pointers.get(&pointer.error_signature_id) {
            self.total_resolutions = self
                .total_resolutions
                .saturating_sub(existing.resolution_ids.len() as u64);
        }

        self.pointers
            .insert(pointer.error_signature_id.clone(), pointer);

        if is_new {
            self.total_patterns += 1;
        }
        self.total_resolutions += new_resolutions;
    }

    pub fn remove_pointer(&mut self, error_signature_id: &str) -> Option<HindsightPointer> {
        if let Some(ptr) = self.pointers.remove(error_signature_id) {
            self.total_patterns = self.total_patterns.saturating_sub(1);
            self.total_resolutions = self
                .total_resolutions
                .saturating_sub(ptr.resolution_ids.len() as u64);
            Some(ptr)
        } else {
            None
        }
    }

    pub fn get_pointers_by_layer(&self, layer: MemoryLayer) -> Vec<&HindsightPointer> {
        self.pointers
            .values()
            .filter(|p| p.source_layer == layer)
            .collect()
    }

    pub fn get_high_success_pointers(
        &self,
        min_rate: f32,
        min_applications: u32,
    ) -> Vec<&HindsightPointer> {
        self.pointers
            .values()
            .filter(|p| p.success_rate >= min_rate && p.application_count >= min_applications)
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePointerMetadata {
    #[serde(rename = "type")]
    pub kind: String,
    pub knowledge_pointer: KnowledgePointer,
    pub tags: Vec<String>,
}

impl Default for KnowledgePointerMetadata {
    fn default() -> Self {
        Self {
            kind: "knowledge_pointer".to_string(),
            knowledge_pointer: KnowledgePointer {
                source_type: KnowledgeType::Adr,
                source_id: String::new(),
                content_hash: String::new(),
                synced_at: 0,
                source_layer: KnowledgeLayer::Tenant,
                is_orphaned: false,
            },
            tags: Vec::new(),
        }
    }
}

pub fn map_layer(knowledge_layer: KnowledgeLayer) -> MemoryLayer {
    match knowledge_layer {
        KnowledgeLayer::Tenant => MemoryLayer::Tenant,
        KnowledgeLayer::Org => MemoryLayer::Org,
        KnowledgeLayer::Team => MemoryLayer::Team,
        KnowledgeLayer::Project => MemoryLayer::Project,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_knowledge_pointer_metadata_default() {
        let metadata = KnowledgePointerMetadata::default();

        assert_eq!(metadata.kind, "knowledge_pointer");
        assert_eq!(metadata.knowledge_pointer.source_type, KnowledgeType::Adr);
        assert!(metadata.knowledge_pointer.source_id.is_empty());
        assert!(metadata.knowledge_pointer.content_hash.is_empty());
        assert_eq!(metadata.knowledge_pointer.synced_at, 0);
        assert_eq!(
            metadata.knowledge_pointer.source_layer,
            KnowledgeLayer::Tenant
        );
        assert!(!metadata.knowledge_pointer.is_orphaned);
        assert!(metadata.tags.is_empty());
    }

    #[test]
    fn test_map_layer_company() {
        assert_eq!(map_layer(KnowledgeLayer::Tenant), MemoryLayer::Tenant);
    }

    #[test]
    fn test_map_layer_org() {
        assert_eq!(map_layer(KnowledgeLayer::Org), MemoryLayer::Org);
    }

    #[test]
    fn test_map_layer_team() {
        assert_eq!(map_layer(KnowledgeLayer::Team), MemoryLayer::Team);
    }

    #[test]
    fn test_map_layer_project() {
        assert_eq!(map_layer(KnowledgeLayer::Project), MemoryLayer::Project);
    }

    #[test]
    fn test_summary_pointer_new() {
        let ptr = SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "content-hash".to_string(),
            "source-hash".to_string(),
            50,
        );

        assert_eq!(ptr.entry_id, "entry-1");
        assert_eq!(ptr.layer, MemoryLayer::Project);
        assert_eq!(ptr.depth, SummaryDepth::Sentence);
        assert_eq!(ptr.content_hash, "content-hash");
        assert_eq!(ptr.source_content_hash, "source-hash");
        assert_eq!(ptr.token_count, 50);
        assert!(!ptr.is_stale);
        assert!(!ptr.personalized);
        assert!(ptr.personalization_context.is_none());
    }

    #[test]
    fn test_summary_pointer_mark_stale() {
        let mut ptr = SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash".to_string(),
            "source".to_string(),
            50,
        );

        assert!(!ptr.is_stale);
        ptr.mark_stale();
        assert!(ptr.is_stale);
    }

    #[test]
    fn test_summary_pointer_needs_update() {
        let ptr = SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash".to_string(),
            "source-hash".to_string(),
            50,
        );

        assert!(!ptr.needs_update("source-hash"));
        assert!(ptr.needs_update("different-hash"));
    }

    #[test]
    fn test_summary_pointer_needs_update_when_stale() {
        let mut ptr = SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash".to_string(),
            "source-hash".to_string(),
            50,
        );

        ptr.mark_stale();
        assert!(ptr.needs_update("source-hash"));
    }

    #[test]
    fn test_summary_pointer_state_set_get() {
        let mut state = SummaryPointerState::default();
        let ptr = SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash".to_string(),
            "source".to_string(),
            50,
        );

        state.set_pointer(ptr.clone());

        let retrieved = state.get_pointer("entry-1", SummaryDepth::Sentence);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().content_hash, "hash");
        assert_eq!(state.total_summaries, 1);
    }

    #[test]
    fn test_summary_pointer_state_multiple_depths() {
        let mut state = SummaryPointerState::default();

        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash1".to_string(),
            "source".to_string(),
            50,
        ));
        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Paragraph,
            "hash2".to_string(),
            "source".to_string(),
            200,
        ));

        assert_eq!(state.total_summaries, 2);
        assert!(
            state
                .get_pointer("entry-1", SummaryDepth::Sentence)
                .is_some()
        );
        assert!(
            state
                .get_pointer("entry-1", SummaryDepth::Paragraph)
                .is_some()
        );
        assert!(
            state
                .get_pointer("entry-1", SummaryDepth::Detailed)
                .is_none()
        );
    }

    #[test]
    fn test_summary_pointer_state_remove() {
        let mut state = SummaryPointerState::default();

        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash".to_string(),
            "source".to_string(),
            50,
        ));

        let removed = state.remove_pointer("entry-1", SummaryDepth::Sentence);
        assert!(removed.is_some());
        assert_eq!(state.total_summaries, 0);
        assert!(
            state
                .get_pointer("entry-1", SummaryDepth::Sentence)
                .is_none()
        );
    }

    #[test]
    fn test_summary_pointer_state_remove_all_for_entry() {
        let mut state = SummaryPointerState::default();

        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash1".to_string(),
            "source".to_string(),
            50,
        ));
        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Paragraph,
            "hash2".to_string(),
            "source".to_string(),
            200,
        ));

        assert_eq!(state.total_summaries, 2);

        let removed = state.remove_all_for_entry("entry-1");
        assert_eq!(removed.len(), 2);
        assert_eq!(state.total_summaries, 0);
    }

    #[test]
    fn test_summary_pointer_state_mark_stale() {
        let mut state = SummaryPointerState::default();

        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash".to_string(),
            "source".to_string(),
            50,
        ));

        assert_eq!(state.stale_count, 0);
        assert!(state.mark_stale("entry-1", SummaryDepth::Sentence));
        assert_eq!(state.stale_count, 1);

        let ptr = state
            .get_pointer("entry-1", SummaryDepth::Sentence)
            .unwrap();
        assert!(ptr.is_stale);
    }

    #[test]
    fn test_summary_pointer_state_mark_all_stale() {
        let mut state = SummaryPointerState::default();

        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash1".to_string(),
            "source".to_string(),
            50,
        ));
        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Paragraph,
            "hash2".to_string(),
            "source".to_string(),
            200,
        ));

        let count = state.mark_all_stale_for_entry("entry-1");
        assert_eq!(count, 2);
        assert_eq!(state.stale_count, 2);
    }

    #[test]
    fn test_summary_pointer_state_get_stale_pointers() {
        let mut state = SummaryPointerState::default();

        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash1".to_string(),
            "source".to_string(),
            50,
        ));
        state.set_pointer(SummaryPointer::new(
            "entry-2".to_string(),
            MemoryLayer::Team,
            SummaryDepth::Paragraph,
            "hash2".to_string(),
            "source".to_string(),
            200,
        ));

        state.mark_stale("entry-1", SummaryDepth::Sentence);

        let stale = state.get_stale_pointers();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].entry_id, "entry-1");
    }

    #[test]
    fn test_summary_pointer_state_get_by_layer() {
        let mut state = SummaryPointerState::default();

        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash1".to_string(),
            "source".to_string(),
            50,
        ));
        state.set_pointer(SummaryPointer::new(
            "entry-2".to_string(),
            MemoryLayer::Team,
            SummaryDepth::Paragraph,
            "hash2".to_string(),
            "source".to_string(),
            200,
        ));

        let project_ptrs = state.get_pointers_for_layer(MemoryLayer::Project);
        assert_eq!(project_ptrs.len(), 1);
        assert_eq!(project_ptrs[0].entry_id, "entry-1");

        let team_ptrs = state.get_pointers_for_layer(MemoryLayer::Team);
        assert_eq!(team_ptrs.len(), 1);
        assert_eq!(team_ptrs[0].entry_id, "entry-2");
    }

    #[test]
    fn test_hindsight_pointer_new() {
        let ptr = HindsightPointer::new("sig-123".to_string(), MemoryLayer::User);

        assert_eq!(ptr.error_signature_id, "sig-123");
        assert!(ptr.memory_entry_id.is_none());
        assert!(ptr.resolution_ids.is_empty());
        assert!(ptr.note_id.is_none());
        assert_eq!(ptr.source_layer, MemoryLayer::User);
        assert_eq!(ptr.success_rate, 0.0);
        assert_eq!(ptr.application_count, 0);
    }

    #[test]
    fn test_hindsight_pointer_add_resolution() {
        let mut ptr = HindsightPointer::new("sig-123".to_string(), MemoryLayer::User);

        ptr.add_resolution("res-1".to_string());
        assert_eq!(ptr.resolution_ids.len(), 1);

        ptr.add_resolution("res-2".to_string());
        assert_eq!(ptr.resolution_ids.len(), 2);

        ptr.add_resolution("res-1".to_string());
        assert_eq!(ptr.resolution_ids.len(), 2);
    }

    #[test]
    fn test_hindsight_pointer_update_success_rate() {
        let mut ptr = HindsightPointer::new("sig-123".to_string(), MemoryLayer::User);

        ptr.update_success_rate(true);
        assert_eq!(ptr.application_count, 1);
        assert_eq!(ptr.success_rate, 1.0);

        ptr.update_success_rate(false);
        assert_eq!(ptr.application_count, 2);
        assert_eq!(ptr.success_rate, 0.5);

        ptr.update_success_rate(true);
        assert_eq!(ptr.application_count, 3);
        assert!((ptr.success_rate - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_hindsight_pointer_state_set_get() {
        let mut state = HindsightPointerState::default();
        let ptr = HindsightPointer::new("sig-123".to_string(), MemoryLayer::User);

        state.set_pointer(ptr);

        let retrieved = state.get_pointer("sig-123");
        assert!(retrieved.is_some());
        assert_eq!(state.total_patterns, 1);
    }

    #[test]
    fn test_hindsight_pointer_state_remove() {
        let mut state = HindsightPointerState::default();
        let mut ptr = HindsightPointer::new("sig-123".to_string(), MemoryLayer::User);
        ptr.add_resolution("res-1".to_string());
        ptr.add_resolution("res-2".to_string());

        state.set_pointer(ptr);
        assert_eq!(state.total_patterns, 1);
        assert_eq!(state.total_resolutions, 2);

        let removed = state.remove_pointer("sig-123");
        assert!(removed.is_some());
        assert_eq!(state.total_patterns, 0);
        assert_eq!(state.total_resolutions, 0);
    }

    #[test]
    fn test_hindsight_pointer_state_get_by_layer() {
        let mut state = HindsightPointerState::default();

        state.set_pointer(HindsightPointer::new(
            "sig-1".to_string(),
            MemoryLayer::User,
        ));
        state.set_pointer(HindsightPointer::new(
            "sig-2".to_string(),
            MemoryLayer::Team,
        ));
        state.set_pointer(HindsightPointer::new(
            "sig-3".to_string(),
            MemoryLayer::User,
        ));

        let user_ptrs = state.get_pointers_by_layer(MemoryLayer::User);
        assert_eq!(user_ptrs.len(), 2);

        let team_ptrs = state.get_pointers_by_layer(MemoryLayer::Team);
        assert_eq!(team_ptrs.len(), 1);
    }

    #[test]
    fn test_hindsight_pointer_state_get_high_success() {
        let mut state = HindsightPointerState::default();

        let mut ptr1 = HindsightPointer::new("sig-1".to_string(), MemoryLayer::User);
        for _ in 0..10 {
            ptr1.update_success_rate(true);
        }

        let mut ptr2 = HindsightPointer::new("sig-2".to_string(), MemoryLayer::User);
        for i in 0..10 {
            ptr2.update_success_rate(i < 5);
        }

        let mut ptr3 = HindsightPointer::new("sig-3".to_string(), MemoryLayer::User);
        ptr3.update_success_rate(true);

        state.set_pointer(ptr1);
        state.set_pointer(ptr2);
        state.set_pointer(ptr3);

        let high_success = state.get_high_success_pointers(0.8, 5);
        assert_eq!(high_success.len(), 1);
        assert_eq!(high_success[0].error_signature_id, "sig-1");
    }

    #[test]
    fn test_summary_pointer_serialization() {
        let ptr = SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash".to_string(),
            "source".to_string(),
            50,
        );

        let json = serde_json::to_string(&ptr).unwrap();
        let deserialized: SummaryPointer = serde_json::from_str(&json).unwrap();
        assert_eq!(ptr, deserialized);
    }

    #[test]
    fn test_hindsight_pointer_serialization() {
        let mut ptr = HindsightPointer::new("sig-123".to_string(), MemoryLayer::User);
        ptr.add_resolution("res-1".to_string());
        ptr.note_id = Some("note-1".to_string());

        let json = serde_json::to_string(&ptr).unwrap();
        let deserialized: HindsightPointer = serde_json::from_str(&json).unwrap();
        assert_eq!(ptr, deserialized);
    }
}

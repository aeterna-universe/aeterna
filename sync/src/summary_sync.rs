use crate::events::{
    InvalidationReason, SummaryCreated, SummaryInvalidated, SummarySyncEvent, SummaryUpdateReason,
    SummaryUpdated,
};
use crate::pointer::{SummaryPointer, SummaryPointerState};
use crate::state::SummarySyncTrigger;
use mk_core::types::{LayerSummary, MemoryLayer, SummaryConfig, SummaryDepth};
use std::collections::HashMap;

/// Return the child layers below `layer` in the knowledge hierarchy
/// (Tenant > Org > Team > Project).  For memory-only layers (Session,
/// User, Agent) there are no children.
fn child_layers(layer: MemoryLayer) -> Vec<MemoryLayer> {
    match layer {
        MemoryLayer::Tenant => vec![MemoryLayer::Org, MemoryLayer::Team, MemoryLayer::Project],
        MemoryLayer::Org => vec![MemoryLayer::Team, MemoryLayer::Project],
        MemoryLayer::Team => vec![MemoryLayer::Project],
        _ => vec![],
    }
}

pub struct SummarySyncResult {
    pub created: Vec<SummarySyncEvent>,
    pub updated: Vec<SummarySyncEvent>,
    pub invalidated: Vec<SummarySyncEvent>,
    pub errors: Vec<SummarySyncError>,
}

impl Default for SummarySyncResult {
    fn default() -> Self {
        Self::new()
    }
}

impl SummarySyncResult {
    pub fn new() -> Self {
        Self {
            created: Vec::new(),
            updated: Vec::new(),
            invalidated: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn total_events(&self) -> usize {
        self.created.len() + self.updated.len() + self.invalidated.len()
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct SummarySyncError {
    pub entry_id: String,
    pub layer: MemoryLayer,
    pub depth: Option<SummaryDepth>,
    pub message: String,
}

pub struct IncrementalSummarySync {
    config_by_layer: HashMap<MemoryLayer, SummaryConfig>,
    change_counts: HashMap<String, u32>,
}

impl Default for IncrementalSummarySync {
    fn default() -> Self {
        Self::new()
    }
}

impl IncrementalSummarySync {
    pub fn new() -> Self {
        Self {
            config_by_layer: HashMap::new(),
            change_counts: HashMap::new(),
        }
    }

    pub fn with_config(mut self, config: SummaryConfig) -> Self {
        self.config_by_layer.insert(config.layer, config);
        self
    }

    pub fn set_config(&mut self, config: SummaryConfig) {
        self.config_by_layer.insert(config.layer, config);
    }

    pub fn get_config(&self, layer: MemoryLayer) -> Option<&SummaryConfig> {
        self.config_by_layer.get(&layer)
    }

    pub fn record_change(&mut self, entry_id: &str) {
        *self.change_counts.entry(entry_id.to_string()).or_insert(0) += 1;
    }

    pub fn reset_change_count(&mut self, entry_id: &str) {
        self.change_counts.remove(entry_id);
    }

    pub fn get_change_count(&self, entry_id: &str) -> u32 {
        self.change_counts.get(entry_id).copied().unwrap_or(0)
    }

    pub fn check_triggers(
        &self,
        entry_id: &str,
        layer: MemoryLayer,
        current_source_hash: &str,
        state: &SummaryPointerState,
        now: i64,
    ) -> Vec<SummarySyncTrigger> {
        let mut triggers = Vec::new();
        let config = match self.config_by_layer.get(&layer) {
            Some(c) => c,
            None => return triggers,
        };

        for depth in &config.depths {
            if let Some(ptr) = state.get_pointer(entry_id, *depth) {
                if ptr.source_content_hash != current_source_hash {
                    triggers.push(SummarySyncTrigger::SourceContentChanged {
                        entry_id: entry_id.to_string(),
                        layer,
                        previous_hash: ptr.source_content_hash.clone(),
                        new_hash: current_source_hash.to_string(),
                    });
                    break;
                }

                if let Some(interval) = config.update_interval_secs {
                    let age = (now - ptr.synced_at) as u64;
                    if age >= interval {
                        triggers.push(SummarySyncTrigger::TimeThresholdExceeded {
                            entry_id: entry_id.to_string(),
                            layer,
                            age_seconds: age,
                            threshold_seconds: interval,
                        });
                        break;
                    }
                }

                if let Some(threshold) = config.update_on_changes {
                    let count = self.get_change_count(entry_id);
                    if count >= threshold {
                        triggers.push(SummarySyncTrigger::ChangeCountExceeded {
                            entry_id: entry_id.to_string(),
                            layer,
                            change_count: count,
                            threshold,
                        });
                        break;
                    }
                }
            } else {
                triggers.push(SummarySyncTrigger::ManualRefresh {
                    entry_id: entry_id.to_string(),
                    layer,
                });
                break;
            }
        }

        triggers
    }

    pub fn sync_summary(
        &mut self,
        entry_id: &str,
        layer: MemoryLayer,
        depth: SummaryDepth,
        summary: &LayerSummary,
        source_content_hash: &str,
        state: &mut SummaryPointerState,
    ) -> Option<SummarySyncEvent> {
        let now = chrono::Utc::now().timestamp();
        let content_hash = utils::compute_content_hash(&summary.content);

        if let Some(existing) = state.get_pointer(entry_id, depth) {
            if existing.content_hash == content_hash && !existing.is_stale {
                return None;
            }

            let reason = if existing.source_content_hash != source_content_hash {
                SummaryUpdateReason::SourceContentChanged
            } else if existing.is_stale {
                SummaryUpdateReason::ScheduledUpdate
            } else {
                SummaryUpdateReason::ManualRefresh
            };

            let previous_hash = existing.content_hash.clone();

            let ptr = SummaryPointer::new(
                entry_id.to_string(),
                layer,
                depth,
                content_hash.clone(),
                source_content_hash.to_string(),
                summary.token_count,
            );
            state.set_pointer(ptr);

            self.reset_change_count(entry_id);

            Some(SummarySyncEvent::Updated(SummaryUpdated {
                entry_id: entry_id.to_string(),
                layer,
                depth,
                previous_hash,
                new_hash: content_hash,
                token_count: summary.token_count,
                updated_at: now,
                reason,
            }))
        } else {
            let ptr = SummaryPointer::new(
                entry_id.to_string(),
                layer,
                depth,
                content_hash.clone(),
                source_content_hash.to_string(),
                summary.token_count,
            );
            state.set_pointer(ptr);

            self.reset_change_count(entry_id);

            Some(SummarySyncEvent::Created(SummaryCreated {
                entry_id: entry_id.to_string(),
                layer,
                depth,
                content_hash,
                token_count: summary.token_count,
                created_at: now,
            }))
        }
    }

    pub fn invalidate_summaries(
        &self,
        entry_id: &str,
        layer: MemoryLayer,
        reason: InvalidationReason,
        state: &mut SummaryPointerState,
    ) -> Option<SummarySyncEvent> {
        let count = state.mark_all_stale_for_entry(entry_id);
        if count == 0 {
            return None;
        }

        let depths: Vec<SummaryDepth> = state
            .pointers
            .get(entry_id)
            .map(|m| m.keys().copied().collect())
            .unwrap_or_default();

        let source_hash = state
            .get_pointer(
                entry_id,
                depths.first().copied().unwrap_or(SummaryDepth::Sentence),
            )
            .map(|p| p.source_content_hash.clone());

        Some(SummarySyncEvent::Invalidated(SummaryInvalidated {
            entry_id: entry_id.to_string(),
            layer,
            depths,
            reason,
            invalidated_at: chrono::Utc::now().timestamp(),
            source_content_hash: source_hash,
        }))
    }

    /// Invalidate summaries for a specific layer only (layer-aware).
    ///
    /// Unlike [`invalidate_summaries`] which marks **all** depths as stale,
    /// this only marks pointers whose `layer` field matches the given layer.
    pub fn invalidate_summaries_for_layer(
        &self,
        entry_id: &str,
        layer: MemoryLayer,
        reason: InvalidationReason,
        state: &mut SummaryPointerState,
    ) -> Option<SummarySyncEvent> {
        let count = state.mark_stale_for_entry_and_layer(entry_id, layer);
        if count == 0 {
            return None;
        }

        let depths: Vec<SummaryDepth> = state
            .pointers
            .get(entry_id)
            .map(|m| {
                m.iter()
                    .filter(|(_, ptr)| ptr.layer == layer && ptr.is_stale)
                    .map(|(d, _)| *d)
                    .collect()
            })
            .unwrap_or_default();

        let source_hash = state
            .get_pointer(
                entry_id,
                depths.first().copied().unwrap_or(SummaryDepth::Sentence),
            )
            .map(|p| p.source_content_hash.clone());

        Some(SummarySyncEvent::Invalidated(SummaryInvalidated {
            entry_id: entry_id.to_string(),
            layer,
            depths,
            reason,
            invalidated_at: chrono::Utc::now().timestamp(),
            source_content_hash: source_hash,
        }))
    }

    /// Invalidate summaries and cascade to child layers in the knowledge
    /// hierarchy.  When a parent layer's summary is invalidated, all
    /// configured child layers for the same entry are also marked stale.
    ///
    /// Returns the events produced (one per invalidated layer).
    pub fn invalidate_summaries_cascade(
        &self,
        entry_id: &str,
        layer: MemoryLayer,
        reason: InvalidationReason,
        state: &mut SummaryPointerState,
    ) -> Vec<SummarySyncEvent> {
        let mut events = Vec::new();

        // Invalidate the requested layer itself.
        if let Some(event) = self.invalidate_summaries_for_layer(entry_id, layer, reason, state) {
            events.push(event);
        }

        // Cascade to child layers below the current one.
        for child_layer in child_layers(layer) {
            if self.config_by_layer.contains_key(&child_layer)
                && let Some(event) = self.invalidate_summaries_for_layer(
                    entry_id,
                    child_layer,
                    InvalidationReason::ParentLayerChanged {
                        parent_layer: layer,
                    },
                    state,
                )
            {
                events.push(event);
            }
        }

        events
    }

    pub fn invalidate_on_source_change(
        &self,
        entry_id: &str,
        layer: MemoryLayer,
        previous_hash: &str,
        new_hash: &str,
        state: &mut SummaryPointerState,
    ) -> Option<SummarySyncEvent> {
        let config = self.config_by_layer.get(&layer)?;

        if config.skip_if_unchanged {
            return None;
        }

        self.invalidate_summaries(
            entry_id,
            layer,
            InvalidationReason::SourceContentChanged {
                previous_hash: previous_hash.to_string(),
                new_hash: new_hash.to_string(),
            },
            state,
        )
    }

    pub fn get_entries_needing_sync(
        &self,
        layer: MemoryLayer,
        source_hashes: &HashMap<String, String>,
        state: &SummaryPointerState,
        now: i64,
    ) -> Vec<String> {
        let config = match self.config_by_layer.get(&layer) {
            Some(c) => c,
            None => return Vec::new(),
        };

        let mut needs_sync = Vec::new();

        for (entry_id, source_hash) in source_hashes {
            let mut entry_needs_sync = false;

            for depth in &config.depths {
                if let Some(ptr) = state.get_pointer(entry_id, *depth) {
                    if ptr.is_stale || ptr.source_content_hash != *source_hash {
                        entry_needs_sync = true;
                        break;
                    }

                    if let Some(interval) = config.update_interval_secs {
                        let age = (now - ptr.synced_at) as u64;
                        if age >= interval {
                            entry_needs_sync = true;
                            break;
                        }
                    }

                    if let Some(threshold) = config.update_on_changes
                        && self.get_change_count(entry_id) >= threshold
                    {
                        entry_needs_sync = true;
                        break;
                    }
                } else {
                    entry_needs_sync = true;
                    break;
                }
            }

            if entry_needs_sync {
                needs_sync.push(entry_id.clone());
            }
        }

        needs_sync
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config(layer: MemoryLayer) -> SummaryConfig {
        SummaryConfig {
            layer,
            update_interval_secs: Some(3600),
            update_on_changes: Some(10),
            skip_if_unchanged: false,
            personalized: false,
            depths: vec![SummaryDepth::Sentence, SummaryDepth::Paragraph],
        }
    }

    fn create_test_summary(depth: SummaryDepth) -> LayerSummary {
        LayerSummary {
            depth,
            content: "Test summary content".to_string(),
            token_count: match depth {
                SummaryDepth::Sentence => 50,
                SummaryDepth::Paragraph => 200,
                SummaryDepth::Detailed => 500,
            },
            generated_at: chrono::Utc::now().timestamp(),
            source_hash: "source-hash".to_string(),
            content_hash: None,
            personalized: false,
            personalization_context: None,
        }
    }

    #[test]
    fn test_incremental_sync_new() {
        let sync = IncrementalSummarySync::new();
        assert!(sync.config_by_layer.is_empty());
        assert!(sync.change_counts.is_empty());
    }

    #[test]
    fn test_incremental_sync_with_config() {
        let config = create_test_config(MemoryLayer::Project);
        let sync = IncrementalSummarySync::new().with_config(config);

        assert!(sync.get_config(MemoryLayer::Project).is_some());
        assert!(sync.get_config(MemoryLayer::Team).is_none());
    }

    #[test]
    fn test_record_change() {
        let mut sync = IncrementalSummarySync::new();

        assert_eq!(sync.get_change_count("entry-1"), 0);
        sync.record_change("entry-1");
        assert_eq!(sync.get_change_count("entry-1"), 1);
        sync.record_change("entry-1");
        assert_eq!(sync.get_change_count("entry-1"), 2);
    }

    #[test]
    fn test_reset_change_count() {
        let mut sync = IncrementalSummarySync::new();

        sync.record_change("entry-1");
        sync.record_change("entry-1");
        assert_eq!(sync.get_change_count("entry-1"), 2);

        sync.reset_change_count("entry-1");
        assert_eq!(sync.get_change_count("entry-1"), 0);
    }

    #[test]
    fn test_check_triggers_source_changed() {
        let config = create_test_config(MemoryLayer::Project);
        let sync = IncrementalSummarySync::new().with_config(config);

        let mut state = SummaryPointerState::default();
        let ptr = SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash".to_string(),
            "old-source".to_string(),
            50,
        );
        state.set_pointer(ptr);

        let triggers = sync.check_triggers(
            "entry-1",
            MemoryLayer::Project,
            "new-source",
            &state,
            chrono::Utc::now().timestamp(),
        );

        assert_eq!(triggers.len(), 1);
        matches!(triggers[0], SummarySyncTrigger::SourceContentChanged { .. });
    }

    #[test]
    fn test_check_triggers_no_existing_pointer() {
        let config = create_test_config(MemoryLayer::Project);
        let sync = IncrementalSummarySync::new().with_config(config);
        let state = SummaryPointerState::default();

        let triggers = sync.check_triggers(
            "entry-1",
            MemoryLayer::Project,
            "source-hash",
            &state,
            chrono::Utc::now().timestamp(),
        );

        assert_eq!(triggers.len(), 1);
        matches!(triggers[0], SummarySyncTrigger::ManualRefresh { .. });
    }

    #[test]
    fn test_check_triggers_time_exceeded() {
        let config = create_test_config(MemoryLayer::Project);
        let sync = IncrementalSummarySync::new().with_config(config);

        let mut state = SummaryPointerState::default();
        let mut ptr = SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash".to_string(),
            "source".to_string(),
            50,
        );
        ptr.synced_at = chrono::Utc::now().timestamp() - 7200;
        state.set_pointer(ptr);

        let triggers = sync.check_triggers(
            "entry-1",
            MemoryLayer::Project,
            "source",
            &state,
            chrono::Utc::now().timestamp(),
        );

        assert_eq!(triggers.len(), 1);
        matches!(
            triggers[0],
            SummarySyncTrigger::TimeThresholdExceeded { .. }
        );
    }

    #[test]
    fn test_sync_summary_create_new() {
        let config = create_test_config(MemoryLayer::Project);
        let mut sync = IncrementalSummarySync::new().with_config(config);
        let mut state = SummaryPointerState::default();

        let summary = create_test_summary(SummaryDepth::Sentence);
        let event = sync.sync_summary(
            "entry-1",
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            &summary,
            "source-hash",
            &mut state,
        );

        assert!(event.is_some());
        matches!(event.unwrap(), SummarySyncEvent::Created(_));
        assert_eq!(state.total_summaries, 1);
    }

    #[test]
    fn test_sync_summary_update_existing() {
        let config = create_test_config(MemoryLayer::Project);
        let mut sync = IncrementalSummarySync::new().with_config(config);
        let mut state = SummaryPointerState::default();

        let ptr = SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "old-hash".to_string(),
            "old-source".to_string(),
            40,
        );
        state.set_pointer(ptr);

        let summary = create_test_summary(SummaryDepth::Sentence);
        let event = sync.sync_summary(
            "entry-1",
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            &summary,
            "new-source",
            &mut state,
        );

        assert!(event.is_some());
        matches!(event.unwrap(), SummarySyncEvent::Updated(_));
    }

    #[test]
    fn test_sync_summary_no_change() {
        let config = create_test_config(MemoryLayer::Project);
        let mut sync = IncrementalSummarySync::new().with_config(config);
        let mut state = SummaryPointerState::default();

        let summary = create_test_summary(SummaryDepth::Sentence);
        let content_hash = utils::compute_content_hash(&summary.content);

        let ptr = SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            content_hash,
            "source-hash".to_string(),
            50,
        );
        state.set_pointer(ptr);

        let event = sync.sync_summary(
            "entry-1",
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            &summary,
            "source-hash",
            &mut state,
        );

        assert!(event.is_none());
    }

    #[test]
    fn test_invalidate_summaries() {
        let config = create_test_config(MemoryLayer::Project);
        let sync = IncrementalSummarySync::new().with_config(config);
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

        let event = sync.invalidate_summaries(
            "entry-1",
            MemoryLayer::Project,
            InvalidationReason::ManualInvalidation,
            &mut state,
        );

        assert!(event.is_some());
        assert_eq!(state.stale_count, 2);

        if let Some(SummarySyncEvent::Invalidated(inv)) = event {
            assert_eq!(inv.entry_id, "entry-1");
            assert_eq!(inv.depths.len(), 2);
        } else {
            panic!("Expected Invalidated event");
        }
    }

    #[test]
    fn test_invalidate_on_source_change() {
        let config = create_test_config(MemoryLayer::Project);
        let sync = IncrementalSummarySync::new().with_config(config);
        let mut state = SummaryPointerState::default();

        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash".to_string(),
            "old-source".to_string(),
            50,
        ));

        let event = sync.invalidate_on_source_change(
            "entry-1",
            MemoryLayer::Project,
            "old-source",
            "new-source",
            &mut state,
        );

        assert!(event.is_some());
        assert_eq!(state.stale_count, 1);
    }

    #[test]
    fn test_get_entries_needing_sync() {
        let config = create_test_config(MemoryLayer::Project);
        let sync = IncrementalSummarySync::new().with_config(config);
        let mut state = SummaryPointerState::default();

        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash".to_string(),
            "source-1".to_string(),
            50,
        ));

        let mut source_hashes = HashMap::new();
        source_hashes.insert("entry-1".to_string(), "source-1".to_string());
        source_hashes.insert("entry-2".to_string(), "source-2".to_string());
        source_hashes.insert("entry-3".to_string(), "new-source".to_string());

        let needs_sync = sync.get_entries_needing_sync(
            MemoryLayer::Project,
            &source_hashes,
            &state,
            chrono::Utc::now().timestamp(),
        );

        assert!(needs_sync.contains(&"entry-2".to_string()));
        assert!(needs_sync.contains(&"entry-3".to_string()));
    }

    #[test]
    fn test_summary_sync_result() {
        let mut result = SummarySyncResult::new();
        assert_eq!(result.total_events(), 0);
        assert!(!result.has_errors());

        result
            .created
            .push(SummarySyncEvent::Created(SummaryCreated {
                entry_id: "entry-1".to_string(),
                layer: MemoryLayer::Project,
                depth: SummaryDepth::Sentence,
                content_hash: "hash".to_string(),
                token_count: 50,
                created_at: 0,
            }));

        assert_eq!(result.total_events(), 1);

        result.errors.push(SummarySyncError {
            entry_id: "entry-2".to_string(),
            layer: MemoryLayer::Project,
            depth: None,
            message: "Test error".to_string(),
        });

        assert!(result.has_errors());
    }

    #[test]
    fn test_child_layers_company() {
        let children = child_layers(MemoryLayer::Tenant);
        assert_eq!(
            children,
            vec![MemoryLayer::Org, MemoryLayer::Team, MemoryLayer::Project]
        );
    }

    #[test]
    fn test_child_layers_org() {
        let children = child_layers(MemoryLayer::Org);
        assert_eq!(children, vec![MemoryLayer::Team, MemoryLayer::Project]);
    }

    #[test]
    fn test_child_layers_team() {
        let children = child_layers(MemoryLayer::Team);
        assert_eq!(children, vec![MemoryLayer::Project]);
    }

    #[test]
    fn test_child_layers_project_has_none() {
        let children = child_layers(MemoryLayer::Project);
        assert!(children.is_empty());
    }

    #[test]
    fn test_child_layers_session_has_none() {
        let children = child_layers(MemoryLayer::Session);
        assert!(children.is_empty());
    }

    #[test]
    fn test_invalidate_summaries_cascade_propagates_to_children() {
        let sync = IncrementalSummarySync::new()
            .with_config(create_test_config(MemoryLayer::Org))
            .with_config(create_test_config(MemoryLayer::Team))
            .with_config(create_test_config(MemoryLayer::Project));

        let mut state = SummaryPointerState::default();

        // Set pointers at Org and Project layers (use different depths
        // because the state is keyed by (entry_id, depth)).
        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Org,
            SummaryDepth::Sentence,
            "hash-org".to_string(),
            "source".to_string(),
            50,
        ));
        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Paragraph,
            "hash-proj".to_string(),
            "source".to_string(),
            200,
        ));

        let events = sync.invalidate_summaries_cascade(
            "entry-1",
            MemoryLayer::Org,
            InvalidationReason::ManualInvalidation,
            &mut state,
        );

        // Should produce at least the Org event plus cascaded child events.
        assert!(
            events.len() >= 2,
            "Expected cascade to produce >=2 events, got {}",
            events.len()
        );

        // The first event should be the Org layer invalidation.
        if let SummarySyncEvent::Invalidated(ref inv) = events[0] {
            assert_eq!(inv.layer, MemoryLayer::Org);
        } else {
            panic!("Expected Invalidated event for Org layer");
        }

        // One of the events should target the Project layer via cascade.
        let has_project = events.iter().any(|e| {
            matches!(
                e,
                SummarySyncEvent::Invalidated(inv) if inv.layer == MemoryLayer::Project
            )
        });
        assert!(has_project, "Cascade should have invalidated Project layer");
    }

    #[test]
    fn test_invalidate_summaries_cascade_no_children_when_leaf() {
        let sync =
            IncrementalSummarySync::new().with_config(create_test_config(MemoryLayer::Project));

        let mut state = SummaryPointerState::default();
        state.set_pointer(SummaryPointer::new(
            "entry-1".to_string(),
            MemoryLayer::Project,
            SummaryDepth::Sentence,
            "hash".to_string(),
            "source".to_string(),
            50,
        ));

        let events = sync.invalidate_summaries_cascade(
            "entry-1",
            MemoryLayer::Project,
            InvalidationReason::ManualInvalidation,
            &mut state,
        );

        // Only the Project layer itself should be invalidated.
        assert_eq!(events.len(), 1);
        if let SummarySyncEvent::Invalidated(ref inv) = events[0] {
            assert_eq!(inv.layer, MemoryLayer::Project);
        }
    }
}

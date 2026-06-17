use metrics::{counter, gauge, histogram};

pub struct KnowledgeTelemetry;

impl KnowledgeTelemetry {
    pub fn record_operation(&self, operation: &str, status: &str) {
        // GIVEN an operation and status
        // WHEN recording metrics
        // THEN increment the operation counter with status label

        counter!("knowledge_operations_total", "operation" => operation.to_string(), "status" => status.to_string()).increment(1);
    }

    pub fn record_violation(&self, layer: &str, severity: &str) {
        // GIVEN a layer and severity
        // WHEN recording metrics
        // THEN increment the violation counter with layer and severity labels

        counter!("knowledge_violations_total", "layer" => layer.to_string(), "severity" => severity.to_string()).increment(1);
    }

    pub fn record_summary_generation(
        &self,
        depth: &str,
        status: &str,
        tokens_used: u32,
        latency_ms: f64,
    ) {
        counter!("cca_summary_generation_total", "depth" => depth.to_string(), "status" => status.to_string()).increment(1);
        histogram!("cca_summary_generation_tokens", "depth" => depth.to_string())
            .record(tokens_used as f64);
        histogram!("cca_summary_generation_latency_ms", "depth" => depth.to_string())
            .record(latency_ms);
    }

    pub fn record_note_distillation(&self, status: &str, events_count: usize, latency_ms: f64) {
        counter!("cca_note_distillation_total", "status" => status.to_string()).increment(1);
        histogram!("cca_note_distillation_events").record(events_count as f64);
        histogram!("cca_note_distillation_latency_ms").record(latency_ms);
    }

    pub fn record_hindsight_query(&self, status: &str, patterns_found: usize, latency_ms: f64) {
        counter!("cca_hindsight_query_total", "status" => status.to_string()).increment(1);
        histogram!("cca_hindsight_query_patterns_found").record(patterns_found as f64);
        histogram!("cca_hindsight_query_latency_ms").record(latency_ms);
    }

    pub fn record_meta_agent_loop(
        &self,
        phase: &str,
        status: &str,
        iteration: u32,
        latency_ms: f64,
    ) {
        counter!("cca_meta_agent_loop_total", "phase" => phase.to_string(), "status" => status.to_string()).increment(1);
        gauge!("cca_meta_agent_loop_iteration", "phase" => phase.to_string()).set(iteration as f64);
        histogram!("cca_meta_agent_loop_latency_ms", "phase" => phase.to_string())
            .record(latency_ms);
    }

    pub fn record_context_assembly(
        &self,
        status: &str,
        layers_included: usize,
        tokens_used: u32,
        latency_ms: f64,
    ) {
        counter!("cca_context_assembly_total", "status" => status.to_string()).increment(1);
        histogram!("cca_context_assembly_layers").record(layers_included as f64);
        histogram!("cca_context_assembly_tokens").record(tokens_used as f64);
        histogram!("cca_context_assembly_latency_ms").record(latency_ms);
    }
    // ── Task 11.1 — Promotion lifecycle metrics ──────────────────────────────

    /// Record a new promotion request being submitted.
    /// Labels: source_layer, target_layer.
    pub fn record_promotion_requested(&self, source_layer: &str, target_layer: &str) {
        counter!(
            "knowledge_promotion_requests_total",
            "source_layer" => source_layer.to_string(),
            "target_layer" => target_layer.to_string()
        )
        .increment(1);
    }

    /// Record a promotion request being approved.
    /// Tracks both event count and end-to-end approval latency (ms from request
    /// creation to approval).
    pub fn record_promotion_approved(&self, target_layer: &str, latency_ms: f64) {
        counter!(
            "knowledge_promotion_approvals_total",
            "target_layer" => target_layer.to_string()
        )
        .increment(1);
        histogram!(
            "knowledge_promotion_approval_latency_ms",
            "target_layer" => target_layer.to_string()
        )
        .record(latency_ms);
    }

    /// Record a promotion request being rejected.
    /// `reason_category` is a short coarse bucket (e.g. "policy", "stale", "manual").
    pub fn record_promotion_rejected(&self, target_layer: &str, reason_category: &str) {
        counter!(
            "knowledge_promotion_rejections_total",
            "target_layer" => target_layer.to_string(),
            "reason_category" => reason_category.to_string()
        )
        .increment(1);
    }

    /// Record a promotion request being retargeted to a new layer.
    pub fn record_promotion_retargeted(&self, old_layer: &str, new_layer: &str) {
        counter!(
            "knowledge_promotion_retargets_total",
            "old_layer" => old_layer.to_string(),
            "new_layer" => new_layer.to_string()
        )
        .increment(1);
    }

    /// Record a promotion conflict (another promotion for the same source is
    /// already Approved or Applied).
    /// `conflict_type` example values: "parallel_approved", "parallel_applied".
    pub fn record_promotion_conflict(&self, conflict_type: &str) {
        counter!(
            "knowledge_promotion_conflicts_total",
            "conflict_type" => conflict_type.to_string()
        )
        .increment(1);
    }

    // ── Task 11.3 — Alert-grade counters ─────────────────────────────────────
    //
    // These counters are intended to drive Prometheus/Alertmanager rules.
    // Any increment above zero in a short window (e.g. 5 min) should trigger
    // an alert.  See docs/guides/knowledge-promotion-alerting.md.

    /// Increment when `apply_promotion` fails for any reason other than an
    /// expected state-machine error (e.g. a storage write error).
    pub fn record_promotion_apply_failed(&self, reason: &str) {
        counter!(
            "knowledge_promotion_apply_failed_total",
            "reason" => reason.to_string()
        )
        .increment(1);
    }

    /// Increment when `NotificationService::notify_promotion` returns an error.
    pub fn record_notification_delivery_failed(&self, event_type: &str) {
        counter!(
            "knowledge_notification_delivery_failed_total",
            "event_type" => event_type.to_string()
        )
        .increment(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_operation() {
        let telemetry = KnowledgeTelemetry;
        telemetry.record_operation("read", "success");
        telemetry.record_operation("write", "failure");
        telemetry.record_operation("delete", "success");
    }

    #[test]
    fn test_record_violation() {
        let telemetry = KnowledgeTelemetry;
        telemetry.record_violation("team", "warn");
        telemetry.record_violation("project", "block");
        telemetry.record_violation("org", "info");
    }

    #[test]
    fn test_record_summary_generation() {
        let telemetry = KnowledgeTelemetry;
        telemetry.record_summary_generation("sentence", "success", 45, 125.5);
        telemetry.record_summary_generation("paragraph", "failure", 0, 250.0);
    }

    #[test]
    fn test_record_note_distillation() {
        let telemetry = KnowledgeTelemetry;
        telemetry.record_note_distillation("success", 5, 320.0);
        telemetry.record_note_distillation("failure", 0, 100.0);
    }

    #[test]
    fn test_record_hindsight_query() {
        let telemetry = KnowledgeTelemetry;
        telemetry.record_hindsight_query("success", 3, 45.5);
        telemetry.record_hindsight_query("failure", 0, 20.0);
    }

    #[test]
    fn test_record_meta_agent_loop() {
        let telemetry = KnowledgeTelemetry;
        telemetry.record_meta_agent_loop("build", "success", 1, 1500.0);
        telemetry.record_meta_agent_loop("test", "failure", 2, 800.0);
    }

    #[test]
    fn test_record_context_assembly() {
        let telemetry = KnowledgeTelemetry;
        telemetry.record_context_assembly("success", 3, 1200, 85.5);
        telemetry.record_context_assembly("failure", 0, 0, 10.0);
    }
    #[test]
    fn test_record_promotion_requested() {
        let t = KnowledgeTelemetry;
        t.record_promotion_requested("team", "org");
        t.record_promotion_requested("project", "team");
    }

    #[test]
    fn test_record_promotion_approved() {
        let t = KnowledgeTelemetry;
        t.record_promotion_approved("org", 1234.5);
        t.record_promotion_approved("tenant", 5678.9);
    }

    #[test]
    fn test_record_promotion_rejected() {
        let t = KnowledgeTelemetry;
        t.record_promotion_rejected("org", "policy");
        t.record_promotion_rejected("team", "stale");
        t.record_promotion_rejected("team", "manual");
    }

    #[test]
    fn test_record_promotion_retargeted() {
        let t = KnowledgeTelemetry;
        t.record_promotion_retargeted("org", "team");
    }

    #[test]
    fn test_record_promotion_conflict() {
        let t = KnowledgeTelemetry;
        t.record_promotion_conflict("parallel_approved");
        t.record_promotion_conflict("parallel_applied");
    }

    #[test]
    fn test_record_promotion_apply_failed() {
        let t = KnowledgeTelemetry;
        t.record_promotion_apply_failed("storage_error");
    }

    #[test]
    fn test_record_notification_delivery_failed() {
        let t = KnowledgeTelemetry;
        t.record_notification_delivery_failed("KnowledgePromotionApproved");
    }
}

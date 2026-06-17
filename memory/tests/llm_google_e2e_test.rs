// E2E tests for GoogleLlmService against live Vertex AI (Gemini).
//
// # Setup
//
// Requires a GCP project with the Vertex AI generative language API enabled.
//
// 1. Enable the API:
//    gcloud services enable aiplatform.googleapis.com --project=$GCP_PROJECT_ID
//
// 2. Grant the service account the `Vertex AI User` IAM role.
//
// 3. Export environment variables:
//    ```sh
//    export AETERNA_GOOGLE_PROJECT_ID="my-gcp-project"
//    export AETERNA_GOOGLE_LOCATION="us-central1"
//    export AETERNA_GOOGLE_MODEL="gemini-2.5-flash"
//    # Authentication: ADC or manual token
//    export GOOGLE_APPLICATION_CREDENTIALS="/path/to/service-account.json"
//    # Or supply a bearer token directly:
//    export GOOGLE_ACCESS_TOKEN="$(gcloud auth print-access-token)"
//    ```
//
// 4. Run:
//    ```sh
//    cargo test -p memory --features google-provider \
//      --test llm_google_e2e_test -- --ignored
//    ```

#[cfg(feature = "google-provider")]
mod google_llm_e2e {
    use memory::llm::google::GoogleLlmService;
    use mk_core::traits::LlmService;
    use mk_core::types::{
        ConstraintOperator, ConstraintSeverity, ConstraintTarget, KnowledgeLayer, Policy,
        PolicyMode, PolicyRule, RuleMergeStrategy, RuleType,
    };
    use std::collections::HashMap;

    fn service_from_env() -> GoogleLlmService {
        let project_id =
            std::env::var("AETERNA_GOOGLE_PROJECT_ID").expect("AETERNA_GOOGLE_PROJECT_ID not set");
        let location =
            std::env::var("AETERNA_GOOGLE_LOCATION").unwrap_or_else(|_| "us-central1".to_string());
        let model = std::env::var("AETERNA_GOOGLE_MODEL")
            .unwrap_or_else(|_| "gemini-2.5-flash".to_string());

        GoogleLlmService::new(project_id, location, model)
    }

    // -------------------------------------------------------------------------
    // Test 4: Live LLM call returns a non-empty text response
    // -------------------------------------------------------------------------
    #[tokio::test]
    #[ignore = "requires GCP project + Vertex AI generative language API - set AETERNA_GOOGLE_PROJECT_ID, AETERNA_GOOGLE_LOCATION, AETERNA_GOOGLE_MODEL, and GOOGLE_ACCESS_TOKEN or ADC"]
    async fn test_gemini_llm_generates_non_empty_response() {
        let svc = service_from_env();

        let response = svc
            .generate("Reply with exactly the word: PONG")
            .await
            .expect("GoogleLlmService::generate must succeed against live Vertex AI");

        assert!(
            !response.trim().is_empty(),
            "LLM response must not be empty"
        );
        assert!(
            response.contains("PONG"),
            "LLM must follow the instruction and include PONG in: {response:?}"
        );
    }

    // -------------------------------------------------------------------------
    // Test 5: LLM multi-turn coherence — second call is independent
    // -------------------------------------------------------------------------
    #[tokio::test]
    #[ignore = "requires GCP project + Vertex AI generative language API - set AETERNA_GOOGLE_PROJECT_ID, AETERNA_GOOGLE_LOCATION, AETERNA_GOOGLE_MODEL, and GOOGLE_ACCESS_TOKEN or ADC"]
    async fn test_gemini_llm_two_sequential_calls_succeed() {
        let svc = service_from_env();

        let r1 = svc
            .generate("What is 2 + 2? Reply with just the number.")
            .await
            .expect("first LLM call must succeed");

        let r2 = svc
            .generate("What is 3 + 3? Reply with just the number.")
            .await
            .expect("second LLM call must succeed");

        assert!(r1.contains('4'), "first answer must contain 4, got: {r1:?}");
        assert!(
            r2.contains('6'),
            "second answer must contain 6, got: {r2:?}"
        );
    }

    // -------------------------------------------------------------------------
    // Test 6: analyze_drift returns a structured ValidationResult
    // -------------------------------------------------------------------------
    #[tokio::test]
    #[ignore = "requires GCP project + Vertex AI generative language API - set AETERNA_GOOGLE_PROJECT_ID, AETERNA_GOOGLE_LOCATION, AETERNA_GOOGLE_MODEL, and GOOGLE_ACCESS_TOKEN or ADC"]
    async fn test_gemini_analyze_drift_returns_validation_result() {
        let svc = service_from_env();

        let policies = vec![Policy {
            id: "no-lodash".to_string(),
            name: "No Vulnerable Lodash".to_string(),
            description: Some("Block lodash < 4.17.21 due to CVE-2021-23337".to_string()),
            layer: KnowledgeLayer::Tenant,
            mode: PolicyMode::Mandatory,
            merge_strategy: RuleMergeStrategy::Merge,
            rules: vec![PolicyRule {
                id: "dep-lodash".to_string(),
                rule_type: RuleType::default(),
                target: ConstraintTarget::Dependency,
                operator: ConstraintOperator::MustNotUse,
                value: serde_json::json!("lodash < 4.17.21"),
                severity: ConstraintSeverity::Block,
                message: "Must not use lodash < 4.17.21 (CVE-2021-23337)".to_string(),
            }],
            metadata: HashMap::new(),
        }];

        // Content that deliberately violates the policy
        let content = r#"{"dependencies": {"lodash": "^3.10.1"}}"#;

        let result = svc
            .analyze_drift(content, &policies)
            .await
            .expect("analyze_drift must return Ok from live Vertex AI");

        // The LLM is expected to detect the violation.
        // We assert the structure is valid regardless of the LLM's judgement
        // (it may or may not flag it, but must return a parseable ValidationResult).
        if !result.is_valid {
            assert!(
                !result.violations.is_empty(),
                "if is_valid=false, violations array must not be empty"
            );
            for v in &result.violations {
                assert!(
                    !v.message.is_empty(),
                    "each violation must carry a non-empty message"
                );
            }
        }
    }
}

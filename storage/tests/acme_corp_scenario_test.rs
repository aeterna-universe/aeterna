//! Scenario-based integration tests based on the Acme Tenant use case from
//! documentation.
//!
//! This test suite validates the complete organizational hierarchy, governance,
//! RBAC, and approval workflow based on a realistic 300-engineer SaaS platform
//! scenario.
//!
//! Organizational Structure:
//! ```text
//! Acme Tenant (root organization)
//! ├── Platform Engineering (Org)
//! │   ├── API Team (Team)
//! │   │   ├── payments-service (Project)
//! │   │   ├── auth-service (Project)
//! │   │   └── gateway-service (Project)
//! │   └── Data Platform Team (Team)
//! │       ├── analytics-pipeline (Project)
//! │       └── ml-inference (Project)
//! ├── Product Engineering (Org)
//! │   ├── Web Team (Team)
//! │   │   ├── dashboard-ui (Project)
//! │   │   └── admin-portal (Project)
//! │   └── Mobile Team (Team)
//! │       ├── ios-app (Project)
//! │       └── android-app (Project)
//! └── Security (Org)
//!     └── SecOps Team (Team)
//!         └── security-scanner (Project)
//! ```

use chrono::Utc;
use mk_core::traits::StorageBackend;
use mk_core::types::{OrganizationalUnit, RecordSource, TenantContext, TenantId, UnitType, UserId};
use std::collections::HashMap;
use storage::governance::{
    ApprovalMode, AuditFilters, CreateApprovalRequest, CreateDecision, CreateGovernanceRole,
    Decision, GovernanceConfig, GovernanceStorage, GovernanceTemplate, PrincipalType,
    RequestFilters, RequestStatus, RequestType, RiskLevel,
};
use storage::postgres::PostgresBackend;
use testing::{postgres, unique_id};
use uuid::Uuid;

struct AcmeCorpHierarchy {
    tenant_id: TenantId,
    tenant_uuid: Uuid,
    platform_eng_org_id: Uuid,
    product_eng_org_id: Uuid,
    security_org_id: Uuid,
    api_team_id: Uuid,
    data_platform_team_id: Uuid,
    web_team_id: Uuid,
    mobile_team_id: Uuid,
    secops_team_id: Uuid,
    payments_service_id: Uuid,
    auth_service_id: Uuid,
    gateway_service_id: Uuid,
    analytics_pipeline_id: Uuid,
    ml_inference_id: Uuid,
    dashboard_ui_id: Uuid,
    admin_portal_id: Uuid,
    ios_app_id: Uuid,
    android_app_id: Uuid,
    security_scanner_id: Uuid,
}

impl AcmeCorpHierarchy {
    fn new(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            tenant_uuid: Uuid::new_v4(),
            platform_eng_org_id: Uuid::new_v4(),
            product_eng_org_id: Uuid::new_v4(),
            security_org_id: Uuid::new_v4(),
            api_team_id: Uuid::new_v4(),
            data_platform_team_id: Uuid::new_v4(),
            web_team_id: Uuid::new_v4(),
            mobile_team_id: Uuid::new_v4(),
            secops_team_id: Uuid::new_v4(),
            payments_service_id: Uuid::new_v4(),
            auth_service_id: Uuid::new_v4(),
            gateway_service_id: Uuid::new_v4(),
            analytics_pipeline_id: Uuid::new_v4(),
            ml_inference_id: Uuid::new_v4(),
            dashboard_ui_id: Uuid::new_v4(),
            admin_portal_id: Uuid::new_v4(),
            ios_app_id: Uuid::new_v4(),
            android_app_id: Uuid::new_v4(),
            security_scanner_id: Uuid::new_v4(),
        }
    }

    async fn setup(&self, storage: &PostgresBackend) -> Result<(), anyhow::Error> {
        let now = Utc::now();

        let root_org = OrganizationalUnit {
            id: self.tenant_id.to_string(),
            name: "Acme Tenant".to_string(),
            unit_type: UnitType::Organization,
            tenant_id: self.tenant_id.clone(),
            parent_id: None,
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&root_org).await?;

        let platform_eng = OrganizationalUnit {
            id: self.platform_eng_org_id.to_string(),
            name: "Platform Engineering".to_string(),
            unit_type: UnitType::Organization,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.tenant_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&platform_eng).await?;

        let product_eng = OrganizationalUnit {
            id: self.product_eng_org_id.to_string(),
            name: "Product Engineering".to_string(),
            unit_type: UnitType::Organization,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.tenant_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&product_eng).await?;

        let security = OrganizationalUnit {
            id: self.security_org_id.to_string(),
            name: "Security".to_string(),
            unit_type: UnitType::Organization,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.tenant_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&security).await?;

        let api_team = OrganizationalUnit {
            id: self.api_team_id.to_string(),
            name: "API Team".to_string(),
            unit_type: UnitType::Team,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.platform_eng_org_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&api_team).await?;

        let data_platform_team = OrganizationalUnit {
            id: self.data_platform_team_id.to_string(),
            name: "Data Platform Team".to_string(),
            unit_type: UnitType::Team,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.platform_eng_org_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&data_platform_team).await?;

        let web_team = OrganizationalUnit {
            id: self.web_team_id.to_string(),
            name: "Web Team".to_string(),
            unit_type: UnitType::Team,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.product_eng_org_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&web_team).await?;

        let mobile_team = OrganizationalUnit {
            id: self.mobile_team_id.to_string(),
            name: "Mobile Team".to_string(),
            unit_type: UnitType::Team,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.product_eng_org_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&mobile_team).await?;

        let secops_team = OrganizationalUnit {
            id: self.secops_team_id.to_string(),
            name: "SecOps Team".to_string(),
            unit_type: UnitType::Team,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.security_org_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&secops_team).await?;

        let payments_service = OrganizationalUnit {
            id: self.payments_service_id.to_string(),
            name: "payments-service".to_string(),
            unit_type: UnitType::Project,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.api_team_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&payments_service).await?;

        let auth_service = OrganizationalUnit {
            id: self.auth_service_id.to_string(),
            name: "auth-service".to_string(),
            unit_type: UnitType::Project,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.api_team_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&auth_service).await?;

        let gateway_service = OrganizationalUnit {
            id: self.gateway_service_id.to_string(),
            name: "gateway-service".to_string(),
            unit_type: UnitType::Project,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.api_team_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&gateway_service).await?;

        let analytics_pipeline = OrganizationalUnit {
            id: self.analytics_pipeline_id.to_string(),
            name: "analytics-pipeline".to_string(),
            unit_type: UnitType::Project,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.data_platform_team_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&analytics_pipeline).await?;

        let ml_inference = OrganizationalUnit {
            id: self.ml_inference_id.to_string(),
            name: "ml-inference".to_string(),
            unit_type: UnitType::Project,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.data_platform_team_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&ml_inference).await?;

        let dashboard_ui = OrganizationalUnit {
            id: self.dashboard_ui_id.to_string(),
            name: "dashboard-ui".to_string(),
            unit_type: UnitType::Project,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.web_team_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&dashboard_ui).await?;

        let admin_portal = OrganizationalUnit {
            id: self.admin_portal_id.to_string(),
            name: "admin-portal".to_string(),
            unit_type: UnitType::Project,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.web_team_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&admin_portal).await?;

        let ios_app = OrganizationalUnit {
            id: self.ios_app_id.to_string(),
            name: "ios-app".to_string(),
            unit_type: UnitType::Project,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.mobile_team_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&ios_app).await?;

        let android_app = OrganizationalUnit {
            id: self.android_app_id.to_string(),
            name: "android-app".to_string(),
            unit_type: UnitType::Project,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.mobile_team_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&android_app).await?;

        let security_scanner = OrganizationalUnit {
            id: self.security_scanner_id.to_string(),
            name: "security-scanner".to_string(),
            unit_type: UnitType::Project,
            tenant_id: self.tenant_id.clone(),
            parent_id: Some(self.secops_team_id.to_string()),
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            source_owner: RecordSource::Admin,
        };
        storage.create_unit(&security_scanner).await?;

        Ok(())
    }
}

async fn create_test_backend() -> Option<PostgresBackend> {
    let fixture = postgres().await?;
    let backend = PostgresBackend::new(fixture.url()).await.ok()?;
    backend.initialize_schema().await.ok()?;
    Some(backend)
}

#[tokio::test]
async fn test_acme_corp_full_hierarchy_setup() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());

    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let ctx = TenantContext {
        tenant_id: tenant_id.clone(),
        user_id: UserId::new("admin".to_string()).unwrap(),
        agent_id: None,
        roles: Vec::new(),
        target_tenant_id: None,
    };

    let descendants = storage
        .get_descendants(ctx.clone(), &hierarchy.tenant_id.to_string())
        .await
        .unwrap();

    assert_eq!(descendants.len(), 18);

    let orgs: Vec<_> = descendants
        .iter()
        .filter(|u| u.unit_type == UnitType::Organization)
        .collect();
    assert_eq!(orgs.len(), 3);

    let teams: Vec<_> = descendants
        .iter()
        .filter(|u| u.unit_type == UnitType::Team)
        .collect();
    assert_eq!(teams.len(), 5);

    let projects: Vec<_> = descendants
        .iter()
        .filter(|u| u.unit_type == UnitType::Project)
        .collect();
    assert_eq!(projects.len(), 10);
}

#[tokio::test]
async fn test_acme_corp_hierarchy_ancestors_from_project() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let ctx = TenantContext {
        tenant_id: tenant_id.clone(),
        user_id: UserId::new("developer".to_string()).unwrap(),
        agent_id: None,
        roles: Vec::new(),
        target_tenant_id: None,
    };

    let ancestors = storage
        .get_ancestors(ctx.clone(), &hierarchy.payments_service_id.to_string())
        .await
        .unwrap();

    assert_eq!(ancestors.len(), 3);

    let ancestor_ids: Vec<_> = ancestors.iter().map(|u| u.id.clone()).collect();
    assert!(ancestor_ids.contains(&hierarchy.api_team_id.to_string()));
    assert!(ancestor_ids.contains(&hierarchy.platform_eng_org_id.to_string()));
    assert!(ancestor_ids.contains(&hierarchy.tenant_id.to_string()));
}

#[tokio::test]
async fn test_acme_corp_platform_engineering_descendants() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let ctx = TenantContext {
        tenant_id: tenant_id.clone(),
        user_id: UserId::new("architect".to_string()).unwrap(),
        agent_id: None,
        roles: Vec::new(),
        target_tenant_id: None,
    };

    let descendants = storage
        .get_descendants(ctx.clone(), &hierarchy.platform_eng_org_id.to_string())
        .await
        .unwrap();

    assert_eq!(descendants.len(), 7);

    let team_names: Vec<_> = descendants
        .iter()
        .filter(|u| u.unit_type == UnitType::Team)
        .map(|u| u.name.clone())
        .collect();
    assert!(team_names.contains(&"API Team".to_string()));
    assert!(team_names.contains(&"Data Platform Team".to_string()));

    let project_names: Vec<_> = descendants
        .iter()
        .filter(|u| u.unit_type == UnitType::Project)
        .map(|u| u.name.clone())
        .collect();
    assert!(project_names.contains(&"payments-service".to_string()));
    assert!(project_names.contains(&"auth-service".to_string()));
    assert!(project_names.contains(&"gateway-service".to_string()));
    assert!(project_names.contains(&"analytics-pipeline".to_string()));
    assert!(project_names.contains(&"ml-inference".to_string()));
}

#[tokio::test]
async fn test_tenant_isolation_different_tenants_cannot_see_each_other() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant1_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy1 = AcmeCorpHierarchy::new(tenant1_id.clone());
    hierarchy1
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy 1");

    let tenant2_id = TenantId::new(unique_id("other")).unwrap();
    let hierarchy2 = AcmeCorpHierarchy::new(tenant2_id.clone());
    hierarchy2
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy 2");

    let ctx2 = TenantContext {
        tenant_id: tenant2_id.clone(),
        user_id: UserId::new("user".to_string()).unwrap(),
        agent_id: None,
        roles: Vec::new(),
        target_tenant_id: None,
    };

    let ancestors = storage
        .get_ancestors(ctx2.clone(), &hierarchy1.payments_service_id.to_string())
        .await
        .unwrap();

    assert_eq!(ancestors.len(), 0);

    let descendants = storage
        .get_descendants(ctx2.clone(), &hierarchy1.tenant_id.to_string())
        .await
        .unwrap();

    assert_eq!(descendants.len(), 0);
}

#[tokio::test]
async fn test_governance_template_standard_config() {
    let config = GovernanceTemplate::Standard.to_config();

    assert_eq!(config.approval_mode, ApprovalMode::Quorum);
    assert_eq!(config.min_approvers, 2);
    assert_eq!(config.timeout_hours, 72);
    assert!(!config.auto_approve_low_risk);
    assert!(config.escalation_enabled);
}

#[tokio::test]
async fn test_governance_template_strict_config() {
    let config = GovernanceTemplate::Strict.to_config();

    assert_eq!(config.approval_mode, ApprovalMode::Unanimous);
    assert_eq!(config.min_approvers, 3);
    assert_eq!(config.timeout_hours, 24);
    assert!(!config.auto_approve_low_risk);
    assert!(config.escalation_enabled);
    assert_eq!(config.escalation_timeout_hours, 12);
}

#[tokio::test]
async fn test_governance_template_permissive_config() {
    let config = GovernanceTemplate::Permissive.to_config();

    assert_eq!(config.approval_mode, ApprovalMode::Single);
    assert_eq!(config.min_approvers, 1);
    assert_eq!(config.timeout_hours, 168);
    assert!(config.auto_approve_low_risk);
    assert!(!config.escalation_enabled);
}

#[tokio::test]
async fn test_governance_template_descriptions() {
    assert!(
        GovernanceTemplate::Standard
            .description()
            .contains("quorum")
    );
    assert!(
        GovernanceTemplate::Strict
            .description()
            .contains("unanimous")
    );
    assert!(
        GovernanceTemplate::Permissive
            .description()
            .contains("single")
    );
}

#[tokio::test]
async fn test_governance_template_all() {
    let all = GovernanceTemplate::all();
    assert_eq!(all.len(), 3);
    assert!(all.contains(&GovernanceTemplate::Standard));
    assert!(all.contains(&GovernanceTemplate::Strict));
    assert!(all.contains(&GovernanceTemplate::Permissive));
}

#[tokio::test]
async fn test_governance_config_upsert_and_retrieve() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let company_config = GovernanceConfig {
        id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: None,
        team_id: None,
        project_id: None,
        approval_mode: ApprovalMode::Quorum,
        min_approvers: 2,
        timeout_hours: 72,
        auto_approve_low_risk: false,
        escalation_enabled: true,
        escalation_timeout_hours: 48,
        escalation_contact: Some("security@acme.corp".to_string()),
        policy_settings: serde_json::json!({"require_approval": true, "min_approvers": 2}),
        knowledge_settings: serde_json::json!({"require_approval": true, "min_approvers": 1}),
        memory_settings: serde_json::json!({"require_approval": false}),
    };

    let config_id = governance.upsert_config(&company_config).await.unwrap();
    assert!(!config_id.is_nil());
}

#[tokio::test]
async fn test_role_assignment_admin_at_company_level() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let admin_user_id = Uuid::new_v4();
    let granter_id = Uuid::new_v4();

    let role = CreateGovernanceRole {
        principal_type: PrincipalType::User,
        principal_id: admin_user_id,
        role: "Admin".to_string(),
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: None,
        team_id: None,
        project_id: None,
        granted_by: granter_id,
        expires_at: None,
    };

    let role_id = governance.assign_role(&role).await.unwrap();
    assert!(!role_id.is_nil());

    let roles = governance
        .list_roles(Some(hierarchy.tenant_uuid), None, None)
        .await
        .unwrap();

    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].role, "Admin");
    assert_eq!(roles[0].principal_id, admin_user_id);
}

#[tokio::test]
async fn test_role_assignment_architect_at_org_level() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let architect_user_id = Uuid::new_v4();
    let granter_id = Uuid::new_v4();

    let role = CreateGovernanceRole {
        principal_type: PrincipalType::User,
        principal_id: architect_user_id,
        role: "Architect".to_string(),
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: Some(hierarchy.platform_eng_org_id),
        team_id: None,
        project_id: None,
        granted_by: granter_id,
        expires_at: None,
    };

    let role_id = governance.assign_role(&role).await.unwrap();
    assert!(!role_id.is_nil());

    let roles = governance
        .list_roles(None, Some(hierarchy.platform_eng_org_id), None)
        .await
        .unwrap();

    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].role, "Architect");
}

#[tokio::test]
async fn test_role_assignment_techlead_at_team_level() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let techlead_user_id = Uuid::new_v4();
    let granter_id = Uuid::new_v4();

    let role = CreateGovernanceRole {
        principal_type: PrincipalType::User,
        principal_id: techlead_user_id,
        role: "TechLead".to_string(),
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: Some(hierarchy.platform_eng_org_id),
        team_id: Some(hierarchy.api_team_id),
        project_id: None,
        granted_by: granter_id,
        expires_at: None,
    };

    let role_id = governance.assign_role(&role).await.unwrap();
    assert!(!role_id.is_nil());

    let roles = governance
        .list_roles(None, None, Some(hierarchy.api_team_id))
        .await
        .unwrap();

    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].role, "TechLead");
}

#[tokio::test]
async fn test_role_assignment_developer_at_project_level() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let developer_user_id = Uuid::new_v4();
    let granter_id = Uuid::new_v4();

    let role = CreateGovernanceRole {
        principal_type: PrincipalType::User,
        principal_id: developer_user_id,
        role: "Developer".to_string(),
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: Some(hierarchy.platform_eng_org_id),
        team_id: Some(hierarchy.api_team_id),
        project_id: Some(hierarchy.payments_service_id),
        granted_by: granter_id,
        expires_at: None,
    };

    let role_id = governance.assign_role(&role).await.unwrap();
    assert!(!role_id.is_nil());
}

#[tokio::test]
async fn test_role_assignment_agent_principal_type() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let agent_id = Uuid::new_v4();
    let granter_id = Uuid::new_v4();

    let role = CreateGovernanceRole {
        principal_type: PrincipalType::Agent,
        principal_id: agent_id,
        role: "Agent".to_string(),
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: Some(hierarchy.platform_eng_org_id),
        team_id: Some(hierarchy.api_team_id),
        project_id: Some(hierarchy.payments_service_id),
        granted_by: granter_id,
        expires_at: None,
    };

    let role_id = governance.assign_role(&role).await.unwrap();
    assert!(!role_id.is_nil());
}

#[tokio::test]
async fn test_role_revocation() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let user_id = Uuid::new_v4();
    let granter_id = Uuid::new_v4();
    let revoker_id = Uuid::new_v4();

    let role = CreateGovernanceRole {
        principal_type: PrincipalType::User,
        principal_id: user_id,
        role: "Developer".to_string(),
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: None,
        team_id: None,
        project_id: None,
        granted_by: granter_id,
        expires_at: None,
    };

    governance.assign_role(&role).await.unwrap();

    let roles_before = governance
        .list_roles(Some(hierarchy.tenant_uuid), None, None)
        .await
        .unwrap();
    assert_eq!(roles_before.len(), 1);

    governance
        .revoke_role(user_id, "Developer", revoker_id)
        .await
        .unwrap();

    let roles_after = governance
        .list_roles(Some(hierarchy.tenant_uuid), None, None)
        .await
        .unwrap();
    assert_eq!(roles_after.len(), 0);
}

#[tokio::test]
async fn test_multiple_roles_same_user_different_scopes() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let user_id = Uuid::new_v4();
    let granter_id = Uuid::new_v4();

    let company_role = CreateGovernanceRole {
        principal_type: PrincipalType::User,
        principal_id: user_id,
        role: "Developer".to_string(),
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: None,
        team_id: None,
        project_id: None,
        granted_by: granter_id,
        expires_at: None,
    };
    governance.assign_role(&company_role).await.unwrap();

    let team_role = CreateGovernanceRole {
        principal_type: PrincipalType::User,
        principal_id: user_id,
        role: "TechLead".to_string(),
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: Some(hierarchy.platform_eng_org_id),
        team_id: Some(hierarchy.api_team_id),
        project_id: None,
        granted_by: granter_id,
        expires_at: None,
    };
    governance.assign_role(&team_role).await.unwrap();

    let company_roles = governance
        .list_roles(Some(hierarchy.tenant_uuid), None, None)
        .await
        .unwrap();
    assert!(!company_roles.is_empty());

    let team_roles = governance
        .list_roles(None, None, Some(hierarchy.api_team_id))
        .await
        .unwrap();
    assert_eq!(team_roles.len(), 1);
    assert_eq!(team_roles[0].role, "TechLead");
}

#[tokio::test]
async fn test_approval_request_creation_policy_change() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let requestor_id = Uuid::new_v4();

    let request = CreateApprovalRequest {
        request_type: RequestType::Policy,
        target_type: "policy".to_string(),
        target_id: Some("security-baseline".to_string()),
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: None,
        team_id: None,
        project_id: None,
        title: "Add lodash vulnerability check".to_string(),
        description: Some("Block lodash versions < 4.17.21 due to CVE-2021-23337".to_string()),
        payload: serde_json::json!({
            "rule": {
                "type": "MustNotUse",
                "target": "lodash < 4.17.21",
                "severity": "Block"
            }
        }),
        risk_level: RiskLevel::High,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: Some("security@acme.corp".to_string()),
        required_approvals: 2,
        timeout_hours: Some(72),
    };

    let created = governance.create_request(&request).await.unwrap();

    assert_eq!(created.request_type, RequestType::Policy);
    assert_eq!(created.status, RequestStatus::Pending);
    assert_eq!(created.required_approvals, 2);
    assert_eq!(created.current_approvals, 0);
    assert!(created.request_number.starts_with("REQ-"));
}

#[tokio::test]
async fn test_approval_request_for_knowledge_change() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let requestor_id = Uuid::new_v4();

    let request = CreateApprovalRequest {
        request_type: RequestType::Knowledge,
        target_type: "adr".to_string(),
        target_id: Some("ADR-042".to_string()),
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: Some(hierarchy.platform_eng_org_id),
        team_id: None,
        project_id: None,
        title: "ADR-042: Use PostgreSQL for new services".to_string(),
        description: Some("Standardize on PostgreSQL for all new backend services".to_string()),
        payload: serde_json::json!({
            "decision": "Use PostgreSQL",
            "status": "proposed"
        }),
        risk_level: RiskLevel::Medium,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: Some("architect@acme.corp".to_string()),
        required_approvals: 1,
        timeout_hours: Some(168),
    };

    let created = governance.create_request(&request).await.unwrap();

    assert_eq!(created.request_type, RequestType::Knowledge);
    assert_eq!(created.risk_level, RiskLevel::Medium);
}

#[tokio::test]
async fn test_approval_decision_approve() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let requestor_id = Uuid::new_v4();
    let approver_id = Uuid::new_v4();

    let request = CreateApprovalRequest {
        request_type: RequestType::Policy,
        target_type: "policy".to_string(),
        target_id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: None,
        team_id: None,
        project_id: None,
        title: "Test policy change".to_string(),
        description: None,
        payload: serde_json::json!({}),
        risk_level: RiskLevel::Low,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: None,
        required_approvals: 1,
        timeout_hours: Some(24),
    };

    let created = governance.create_request(&request).await.unwrap();

    let decision = CreateDecision {
        request_id: created.id,
        approver_type: PrincipalType::User,
        approver_id,
        approver_email: Some("approver@acme.corp".to_string()),
        decision: Decision::Approve,
        comment: Some("LGTM".to_string()),
    };

    let saved_decision = governance.add_decision(&decision).await.unwrap();

    assert_eq!(saved_decision.decision, Decision::Approve);
    assert_eq!(saved_decision.comment, Some("LGTM".to_string()));

    let decisions = governance.get_decisions(created.id).await.unwrap();
    assert_eq!(decisions.len(), 1);
}

#[tokio::test]
async fn test_approval_decision_reject() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let requestor_id = Uuid::new_v4();
    let approver_id = Uuid::new_v4();

    let request = CreateApprovalRequest {
        request_type: RequestType::Policy,
        target_type: "policy".to_string(),
        target_id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: None,
        team_id: None,
        project_id: None,
        title: "Risky policy change".to_string(),
        description: None,
        payload: serde_json::json!({}),
        risk_level: RiskLevel::Critical,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: None,
        required_approvals: 3,
        timeout_hours: Some(24),
    };

    let created = governance.create_request(&request).await.unwrap();

    let decision = CreateDecision {
        request_id: created.id,
        approver_type: PrincipalType::User,
        approver_id,
        approver_email: None,
        decision: Decision::Reject,
        comment: Some("Too risky without security review".to_string()),
    };

    governance.add_decision(&decision).await.unwrap();

    let rejected = governance
        .reject_request(created.id, "Rejected due to security concerns")
        .await
        .unwrap();

    assert_eq!(rejected.status, RequestStatus::Rejected);
    assert!(rejected.resolution_reason.is_some());
}

#[tokio::test]
async fn test_approval_decision_abstain() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let requestor_id = Uuid::new_v4();
    let approver_id = Uuid::new_v4();

    let request = CreateApprovalRequest {
        request_type: RequestType::Knowledge,
        target_type: "pattern".to_string(),
        target_id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: None,
        team_id: None,
        project_id: None,
        title: "New design pattern".to_string(),
        description: None,
        payload: serde_json::json!({}),
        risk_level: RiskLevel::Low,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: None,
        required_approvals: 2,
        timeout_hours: Some(72),
    };

    let created = governance.create_request(&request).await.unwrap();

    let decision = CreateDecision {
        request_id: created.id,
        approver_type: PrincipalType::User,
        approver_id,
        approver_email: None,
        decision: Decision::Abstain,
        comment: Some("Not familiar with this area".to_string()),
    };

    let saved = governance.add_decision(&decision).await.unwrap();
    assert_eq!(saved.decision, Decision::Abstain);
}

#[tokio::test]
async fn test_approval_request_cancellation() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let requestor_id = Uuid::new_v4();

    let request = CreateApprovalRequest {
        request_type: RequestType::Config,
        target_type: "config".to_string(),
        target_id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: None,
        team_id: None,
        project_id: None,
        title: "Update timeout settings".to_string(),
        description: None,
        payload: serde_json::json!({}),
        risk_level: RiskLevel::Low,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: None,
        required_approvals: 1,
        timeout_hours: Some(24),
    };

    let created = governance.create_request(&request).await.unwrap();

    let cancelled = governance.cancel_request(created.id).await.unwrap();

    assert_eq!(cancelled.status, RequestStatus::Cancelled);
    assert!(cancelled.resolved_at.is_some());
}

#[tokio::test]
async fn test_approval_request_mark_applied() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let requestor_id = Uuid::new_v4();
    let applier_id = Uuid::new_v4();

    let request = CreateApprovalRequest {
        request_type: RequestType::Policy,
        target_type: "policy".to_string(),
        target_id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: None,
        team_id: None,
        project_id: None,
        title: "Approved policy".to_string(),
        description: None,
        payload: serde_json::json!({}),
        risk_level: RiskLevel::Low,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: None,
        required_approvals: 1,
        timeout_hours: Some(24),
    };

    let created = governance.create_request(&request).await.unwrap();

    let applied = governance
        .mark_applied(created.id, applier_id)
        .await
        .unwrap();

    assert!(applied.applied_at.is_some());
    assert_eq!(applied.applied_by, Some(applier_id));
}

#[tokio::test]
async fn test_list_pending_requests_with_filters() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let requestor_id = Uuid::new_v4();

    for i in 0..3 {
        let request = CreateApprovalRequest {
            request_type: RequestType::Policy,
            target_type: "policy".to_string(),
            target_id: None,
            tenant_id: Some(hierarchy.tenant_uuid),
            org_id: None,
            team_id: None,
            project_id: None,
            title: format!("Policy request {}", i),
            description: None,
            payload: serde_json::json!({}),
            risk_level: RiskLevel::Low,
            requestor_type: PrincipalType::User,
            requestor_id,
            requestor_email: None,
            required_approvals: 1,
            timeout_hours: Some(24),
        };
        governance.create_request(&request).await.unwrap();
    }

    let knowledge_request = CreateApprovalRequest {
        request_type: RequestType::Knowledge,
        target_type: "adr".to_string(),
        target_id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: None,
        team_id: None,
        project_id: None,
        title: "Knowledge request".to_string(),
        description: None,
        payload: serde_json::json!({}),
        risk_level: RiskLevel::Low,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: None,
        required_approvals: 1,
        timeout_hours: Some(24),
    };
    governance.create_request(&knowledge_request).await.unwrap();

    let all_pending = governance
        .list_pending_requests(&RequestFilters {
            tenant_id: Some(hierarchy.tenant_uuid),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(all_pending.len(), 4);

    let policy_only = governance
        .list_pending_requests(&RequestFilters {
            request_type: Some(RequestType::Policy),
            tenant_id: Some(hierarchy.tenant_uuid),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(policy_only.len(), 3);

    let limited = governance
        .list_pending_requests(&RequestFilters {
            tenant_id: Some(hierarchy.tenant_uuid),
            limit: Some(2),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(limited.len(), 2);
}

#[tokio::test]
async fn test_get_request_by_id_and_number() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let requestor_id = Uuid::new_v4();

    let request = CreateApprovalRequest {
        request_type: RequestType::Role,
        target_type: "role".to_string(),
        target_id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: None,
        team_id: None,
        project_id: None,
        title: "Role change request".to_string(),
        description: None,
        payload: serde_json::json!({}),
        risk_level: RiskLevel::Medium,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: None,
        required_approvals: 2,
        timeout_hours: Some(48),
    };

    let created = governance.create_request(&request).await.unwrap();

    let by_id = governance.get_request(created.id).await.unwrap();
    assert!(by_id.is_some());
    assert_eq!(by_id.unwrap().id, created.id);

    let by_number = governance
        .get_request_by_number(&created.request_number)
        .await
        .unwrap();
    assert!(by_number.is_some());
    assert_eq!(by_number.unwrap().request_number, created.request_number);

    let not_found = governance.get_request(Uuid::new_v4()).await.unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn test_audit_log_governance_action() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let actor_id = Uuid::new_v4();
    let request_id = Uuid::new_v4();

    let audit_id = governance
        .log_audit(
            "approval_decision",
            Some(request_id),
            Some("policy"),
            Some("security-baseline"),
            PrincipalType::User,
            Some(actor_id),
            Some("admin@acme.corp"),
            serde_json::json!({
                "decision": "approve",
                "comment": "Looks good"
            }),
            None,
        )
        .await
        .unwrap();

    assert!(!audit_id.is_nil());
}

#[tokio::test]
async fn test_audit_log_list_with_filters() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let actor1_id = Uuid::new_v4();
    let actor2_id = Uuid::new_v4();

    governance
        .log_audit(
            "role_assigned",
            None,
            Some("role"),
            Some("Admin"),
            PrincipalType::User,
            Some(actor1_id),
            None,
            serde_json::json!({"role": "Admin"}),
            None,
        )
        .await
        .unwrap();

    governance
        .log_audit(
            "policy_created",
            None,
            Some("policy"),
            Some("new-policy"),
            PrincipalType::User,
            Some(actor2_id),
            None,
            serde_json::json!({"policy": "new-policy"}),
            None,
        )
        .await
        .unwrap();

    governance
        .log_audit(
            "role_assigned",
            None,
            Some("role"),
            Some("Developer"),
            PrincipalType::System,
            None,
            None,
            serde_json::json!({"role": "Developer", "auto": true}),
            None,
        )
        .await
        .unwrap();

    let all_logs = governance
        .list_audit_logs(&AuditFilters {
            action: None,
            actor_id: None,
            target_type: None,
            since: Utc::now() - chrono::Duration::hours(1),
            limit: Some(100),
            acting_as_tenant_id: None,
            via: None,
            client_version: None,
            manifest_hash: None,
            generation: None,
            dry_run: None,
        })
        .await
        .unwrap();
    assert!(all_logs.len() >= 3);

    let role_logs = governance
        .list_audit_logs(&AuditFilters {
            action: Some("role_assigned".to_string()),
            actor_id: None,
            target_type: None,
            since: Utc::now() - chrono::Duration::hours(1),
            limit: Some(100),
            acting_as_tenant_id: None,
            via: None,
            client_version: None,
            manifest_hash: None,
            generation: None,
            dry_run: None,
        })
        .await
        .unwrap();
    assert!(role_logs.len() >= 2);

    let actor1_logs = governance
        .list_audit_logs(&AuditFilters {
            action: None,
            actor_id: Some(actor1_id),
            target_type: None,
            since: Utc::now() - chrono::Duration::hours(1),
            limit: Some(100),
            acting_as_tenant_id: None,
            via: None,
            client_version: None,
            manifest_hash: None,
            generation: None,
            dry_run: None,
        })
        .await
        .unwrap();
    assert_eq!(actor1_logs.len(), 1);
    assert_eq!(actor1_logs[0].action, "role_assigned");
}

#[tokio::test]
async fn test_risk_level_parsing() {
    assert_eq!("low".parse::<RiskLevel>().unwrap(), RiskLevel::Low);
    assert_eq!("medium".parse::<RiskLevel>().unwrap(), RiskLevel::Medium);
    assert_eq!("high".parse::<RiskLevel>().unwrap(), RiskLevel::High);
    assert_eq!(
        "critical".parse::<RiskLevel>().unwrap(),
        RiskLevel::Critical
    );
    assert!("unknown".parse::<RiskLevel>().is_err());
}

#[tokio::test]
async fn test_risk_level_display() {
    assert_eq!(RiskLevel::Low.to_string(), "low");
    assert_eq!(RiskLevel::Medium.to_string(), "medium");
    assert_eq!(RiskLevel::High.to_string(), "high");
    assert_eq!(RiskLevel::Critical.to_string(), "critical");
}

#[tokio::test]
async fn test_request_type_parsing() {
    assert_eq!(
        "policy".parse::<RequestType>().unwrap(),
        RequestType::Policy
    );
    assert_eq!(
        "knowledge".parse::<RequestType>().unwrap(),
        RequestType::Knowledge
    );
    assert_eq!(
        "memory".parse::<RequestType>().unwrap(),
        RequestType::Memory
    );
    assert_eq!("role".parse::<RequestType>().unwrap(), RequestType::Role);
    assert_eq!(
        "config".parse::<RequestType>().unwrap(),
        RequestType::Config
    );
    assert!("unknown".parse::<RequestType>().is_err());
}

#[tokio::test]
async fn test_request_status_parsing() {
    assert_eq!(
        "pending".parse::<RequestStatus>().unwrap(),
        RequestStatus::Pending
    );
    assert_eq!(
        "approved".parse::<RequestStatus>().unwrap(),
        RequestStatus::Approved
    );
    assert_eq!(
        "rejected".parse::<RequestStatus>().unwrap(),
        RequestStatus::Rejected
    );
    assert_eq!(
        "expired".parse::<RequestStatus>().unwrap(),
        RequestStatus::Expired
    );
    assert_eq!(
        "cancelled".parse::<RequestStatus>().unwrap(),
        RequestStatus::Cancelled
    );
    assert!("unknown".parse::<RequestStatus>().is_err());
}

#[tokio::test]
async fn test_principal_type_parsing() {
    assert_eq!(
        "user".parse::<PrincipalType>().unwrap(),
        PrincipalType::User
    );
    assert_eq!(
        "agent".parse::<PrincipalType>().unwrap(),
        PrincipalType::Agent
    );
    assert_eq!(
        "system".parse::<PrincipalType>().unwrap(),
        PrincipalType::System
    );
    assert!("unknown".parse::<PrincipalType>().is_err());
}

#[tokio::test]
async fn test_approval_mode_parsing() {
    assert_eq!(
        "single".parse::<ApprovalMode>().unwrap(),
        ApprovalMode::Single
    );
    assert_eq!(
        "quorum".parse::<ApprovalMode>().unwrap(),
        ApprovalMode::Quorum
    );
    assert_eq!(
        "unanimous".parse::<ApprovalMode>().unwrap(),
        ApprovalMode::Unanimous
    );
    assert!("unknown".parse::<ApprovalMode>().is_err());
}

#[tokio::test]
async fn test_governance_template_parsing() {
    assert_eq!(
        "standard".parse::<GovernanceTemplate>().unwrap(),
        GovernanceTemplate::Standard
    );
    assert_eq!(
        "strict".parse::<GovernanceTemplate>().unwrap(),
        GovernanceTemplate::Strict
    );
    assert_eq!(
        "permissive".parse::<GovernanceTemplate>().unwrap(),
        GovernanceTemplate::Permissive
    );
    assert!("unknown".parse::<GovernanceTemplate>().is_err());
}

#[tokio::test]
async fn test_memory_promotion_request_workflow() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let developer_id = Uuid::new_v4();
    let techlead_id = Uuid::new_v4();

    let request = CreateApprovalRequest {
        request_type: RequestType::Memory,
        target_type: "memory_promotion".to_string(),
        target_id: Some("mem-12345".to_string()),
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: Some(hierarchy.platform_eng_org_id),
        team_id: Some(hierarchy.api_team_id),
        project_id: Some(hierarchy.payments_service_id),
        title: "Promote memory: PostgreSQL timeout configuration".to_string(),
        description: Some(
            "This memory about optimal PostgreSQL timeout settings has high reward and should be \
             promoted to team level"
                .to_string(),
        ),
        payload: serde_json::json!({
            "memory_id": "mem-12345",
            "current_layer": "project",
            "target_layer": "team",
            "reward_score": 0.92,
            "content_summary": "Set PostgreSQL connection timeout to 5s for API services"
        }),
        risk_level: RiskLevel::Low,
        requestor_type: PrincipalType::User,
        requestor_id: developer_id,
        requestor_email: Some("developer@acme.corp".to_string()),
        required_approvals: 1,
        timeout_hours: Some(48),
    };

    let created = governance.create_request(&request).await.unwrap();
    assert_eq!(created.request_type, RequestType::Memory);

    let decision = CreateDecision {
        request_id: created.id,
        approver_type: PrincipalType::User,
        approver_id: techlead_id,
        approver_email: Some("techlead@acme.corp".to_string()),
        decision: Decision::Approve,
        comment: Some("Good insight, promoting to team level".to_string()),
    };

    governance.add_decision(&decision).await.unwrap();

    governance
        .log_audit(
            "memory_promoted",
            Some(created.id),
            Some("memory"),
            Some("mem-12345"),
            PrincipalType::System,
            None,
            None,
            serde_json::json!({
                "from_layer": "project",
                "to_layer": "team",
                "approved_by": techlead_id
            }),
            None,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn test_cross_org_policy_request() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let security_architect_id = Uuid::new_v4();

    let request = CreateApprovalRequest {
        request_type: RequestType::Policy,
        target_type: "policy".to_string(),
        target_id: Some("cross-org-security".to_string()),
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: None,
        team_id: None,
        project_id: None,
        title: "Tenant-wide security policy affecting all orgs".to_string(),
        description: Some(
            "This policy will be enforced across Platform Engineering, Product Engineering, and \
             Security orgs"
                .to_string(),
        ),
        payload: serde_json::json!({
            "affected_orgs": [
                hierarchy.platform_eng_org_id,
                hierarchy.product_eng_org_id,
                hierarchy.security_org_id
            ],
            "rules": [
                {"type": "MustNotUse", "target": "eval()", "severity": "Block"},
                {"type": "MustExist", "target": "SECURITY.md", "severity": "Warn"}
            ]
        }),
        risk_level: RiskLevel::High,
        requestor_type: PrincipalType::User,
        requestor_id: security_architect_id,
        requestor_email: Some("security-architect@acme.corp".to_string()),
        required_approvals: 3,
        timeout_hours: Some(72),
    };

    let created = governance.create_request(&request).await.unwrap();

    assert_eq!(created.required_approvals, 3);
    assert_eq!(created.risk_level, RiskLevel::High);
    assert!(created.org_id.is_none());
}

#[tokio::test]
async fn test_filter_requests_by_org_scope() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let requestor_id = Uuid::new_v4();

    let platform_request = CreateApprovalRequest {
        request_type: RequestType::Policy,
        target_type: "policy".to_string(),
        target_id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: Some(hierarchy.platform_eng_org_id),
        team_id: None,
        project_id: None,
        title: "Platform Engineering policy".to_string(),
        description: None,
        payload: serde_json::json!({}),
        risk_level: RiskLevel::Medium,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: None,
        required_approvals: 2,
        timeout_hours: Some(48),
    };
    governance.create_request(&platform_request).await.unwrap();

    let product_request = CreateApprovalRequest {
        request_type: RequestType::Policy,
        target_type: "policy".to_string(),
        target_id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: Some(hierarchy.product_eng_org_id),
        team_id: None,
        project_id: None,
        title: "Product Engineering policy".to_string(),
        description: None,
        payload: serde_json::json!({}),
        risk_level: RiskLevel::Medium,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: None,
        required_approvals: 2,
        timeout_hours: Some(48),
    };
    governance.create_request(&product_request).await.unwrap();

    let platform_requests = governance
        .list_pending_requests(&RequestFilters {
            org_id: Some(hierarchy.platform_eng_org_id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(platform_requests.len(), 1);
    assert_eq!(
        platform_requests[0].title,
        "Platform Engineering policy".to_string()
    );

    let product_requests = governance
        .list_pending_requests(&RequestFilters {
            org_id: Some(hierarchy.product_eng_org_id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(product_requests.len(), 1);
    assert_eq!(
        product_requests[0].title,
        "Product Engineering policy".to_string()
    );
}

#[tokio::test]
async fn test_filter_requests_by_team_scope() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let requestor_id = Uuid::new_v4();

    let api_team_request = CreateApprovalRequest {
        request_type: RequestType::Knowledge,
        target_type: "pattern".to_string(),
        target_id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: Some(hierarchy.platform_eng_org_id),
        team_id: Some(hierarchy.api_team_id),
        project_id: None,
        title: "API Team pattern".to_string(),
        description: None,
        payload: serde_json::json!({}),
        risk_level: RiskLevel::Low,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: None,
        required_approvals: 1,
        timeout_hours: Some(72),
    };
    governance.create_request(&api_team_request).await.unwrap();

    let web_team_request = CreateApprovalRequest {
        request_type: RequestType::Knowledge,
        target_type: "pattern".to_string(),
        target_id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: Some(hierarchy.product_eng_org_id),
        team_id: Some(hierarchy.web_team_id),
        project_id: None,
        title: "Web Team pattern".to_string(),
        description: None,
        payload: serde_json::json!({}),
        risk_level: RiskLevel::Low,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: None,
        required_approvals: 1,
        timeout_hours: Some(72),
    };
    governance.create_request(&web_team_request).await.unwrap();

    let api_requests = governance
        .list_pending_requests(&RequestFilters {
            team_id: Some(hierarchy.api_team_id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(api_requests.len(), 1);

    let web_requests = governance
        .list_pending_requests(&RequestFilters {
            team_id: Some(hierarchy.web_team_id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(web_requests.len(), 1);
}

#[tokio::test]
async fn test_filter_requests_by_project_scope() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let requestor_id = Uuid::new_v4();

    let payments_request = CreateApprovalRequest {
        request_type: RequestType::Config,
        target_type: "config".to_string(),
        target_id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: Some(hierarchy.platform_eng_org_id),
        team_id: Some(hierarchy.api_team_id),
        project_id: Some(hierarchy.payments_service_id),
        title: "Payments service config".to_string(),
        description: None,
        payload: serde_json::json!({}),
        risk_level: RiskLevel::Medium,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: None,
        required_approvals: 1,
        timeout_hours: Some(24),
    };
    governance.create_request(&payments_request).await.unwrap();

    let auth_request = CreateApprovalRequest {
        request_type: RequestType::Config,
        target_type: "config".to_string(),
        target_id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: Some(hierarchy.platform_eng_org_id),
        team_id: Some(hierarchy.api_team_id),
        project_id: Some(hierarchy.auth_service_id),
        title: "Auth service config".to_string(),
        description: None,
        payload: serde_json::json!({}),
        risk_level: RiskLevel::High,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: None,
        required_approvals: 2,
        timeout_hours: Some(24),
    };
    governance.create_request(&auth_request).await.unwrap();

    let payments_requests = governance
        .list_pending_requests(&RequestFilters {
            project_id: Some(hierarchy.payments_service_id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(payments_requests.len(), 1);
    assert_eq!(payments_requests[0].risk_level, RiskLevel::Medium);

    let auth_requests = governance
        .list_pending_requests(&RequestFilters {
            project_id: Some(hierarchy.auth_service_id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(auth_requests.len(), 1);
    assert_eq!(auth_requests[0].risk_level, RiskLevel::High);
}

#[tokio::test]
async fn test_filter_requests_by_requestor() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("acme")).unwrap();
    let hierarchy = AcmeCorpHierarchy::new(tenant_id.clone());
    hierarchy
        .setup(&storage)
        .await
        .expect("Failed to setup hierarchy");

    let governance = GovernanceStorage::new(storage.pool().clone());

    let alice_id = Uuid::new_v4();
    let bob_id = Uuid::new_v4();

    for i in 0..3 {
        let request = CreateApprovalRequest {
            request_type: RequestType::Policy,
            target_type: "policy".to_string(),
            target_id: None,
            tenant_id: Some(hierarchy.tenant_uuid),
            org_id: None,
            team_id: None,
            project_id: None,
            title: format!("Alice's request {}", i),
            description: None,
            payload: serde_json::json!({}),
            risk_level: RiskLevel::Low,
            requestor_type: PrincipalType::User,
            requestor_id: alice_id,
            requestor_email: Some("alice@acme.corp".to_string()),
            required_approvals: 1,
            timeout_hours: Some(24),
        };
        governance.create_request(&request).await.unwrap();
    }

    let bob_request = CreateApprovalRequest {
        request_type: RequestType::Policy,
        target_type: "policy".to_string(),
        target_id: None,
        tenant_id: Some(hierarchy.tenant_uuid),
        org_id: None,
        team_id: None,
        project_id: None,
        title: "Bob's request".to_string(),
        description: None,
        payload: serde_json::json!({}),
        risk_level: RiskLevel::Low,
        requestor_type: PrincipalType::User,
        requestor_id: bob_id,
        requestor_email: Some("bob@acme.corp".to_string()),
        required_approvals: 1,
        timeout_hours: Some(24),
    };
    governance.create_request(&bob_request).await.unwrap();

    let alice_requests = governance
        .list_pending_requests(&RequestFilters {
            requestor_id: Some(alice_id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(alice_requests.len(), 3);

    let bob_requests = governance
        .list_pending_requests(&RequestFilters {
            requestor_id: Some(bob_id),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(bob_requests.len(), 1);
}

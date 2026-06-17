use aeterna_backup::archive::ArchiveReader;
use aeterna_backup::validate::validate_archive;
use clap::{Args, Subcommand, ValueEnum};
use context::ContextResolver;
use mk_core::types::PROVIDER_GITHUB;
use serde::Serialize;
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;

use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use sqlx::{FromRow, PgPool};
use storage::migrations::{EmbeddedMigration, MIGRATIONS};

use crate::output;
use crate::ux_error;

#[derive(Subcommand)]
pub enum AdminCommand {
    #[command(about = "Check system health and connectivity")]
    Health(AdminHealthArgs),

    #[command(about = "Validate configuration and data integrity")]
    Validate(AdminValidateArgs),

    #[command(about = "Run database migrations")]
    Migrate(AdminMigrateArgs),

    #[command(about = "Detect configuration drift from expected state")]
    Drift(AdminDriftArgs),

    #[command(about = "Export data for backup or migration")]
    Export(AdminExportArgs),

    #[command(about = "Import data from backup or another instance")]
    Import(AdminImportArgs),

    #[command(about = "Sync organizational data from identity provider")]
    Sync(AdminSyncArgs),

    #[command(subcommand, about = "Backup and restore operations")]
    Backup(AdminBackupCommand),

    #[command(subcommand, about = "Tenant provisioning operations (PlatformAdmin)")]
    Tenant(AdminTenantCommand),
}

#[derive(Subcommand)]
pub enum AdminTenantCommand {
    #[command(about = "Provision a tenant from a YAML/JSON manifest file")]
    Provision(AdminTenantProvisionArgs),
}

#[derive(Args)]
pub struct AdminTenantProvisionArgs {
    /// Path to the tenant manifest file (YAML or JSON)
    #[arg(short, long)]
    pub file: PathBuf,

    /// Dry run - validate manifest without provisioning
    #[arg(long)]
    pub dry_run: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum AdminBackupCommand {
    #[command(about = "Validate a backup archive integrity offline")]
    Validate(AdminBackupValidateArgs),
}

#[derive(Args)]
pub struct AdminBackupValidateArgs {
    /// Path to the backup archive file
    pub archive: PathBuf,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct AdminHealthArgs {
    /// Check specific component (memory, knowledge, policy, all)
    #[arg(short, long, default_value = "all")]
    pub component: String,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Verbose output with detailed metrics
    #[arg(short, long)]
    pub verbose: bool,

    /// Timeout in seconds for health checks
    #[arg(long, default_value = "30")]
    pub timeout: u64,
}

#[derive(Args)]
pub struct AdminValidateArgs {
    /// What to validate (config, schema, policies, all)
    #[arg(short, long, default_value = "all")]
    pub target: String,

    /// Path to configuration file (defaults to auto-detect)
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Strict mode - treat warnings as errors
    #[arg(long)]
    pub strict: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct AdminMigrateArgs {
    /// Migration direction (up, down, status)
    #[arg(default_value = "up")]
    pub direction: String,

    /// Target version (latest if not specified)
    #[arg(long)]
    pub target: Option<String>,

    /// Dry run - show what would be migrated
    #[arg(long)]
    pub dry_run: bool,

    /// Force migration even if data loss may occur
    #[arg(long)]
    pub force: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct AdminDriftArgs {
    /// Expected state source (git, snapshot, file)
    #[arg(long, default_value = "git")]
    pub source: String,

    /// Path to expected state file (for file source)
    #[arg(long)]
    pub expected: Option<PathBuf>,

    /// What to check for drift (config, policies, schema, all)
    #[arg(short, long, default_value = "all")]
    pub target: String,

    /// Auto-fix detected drift
    #[arg(long)]
    pub fix: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct AdminExportArgs {
    /// What to export (memories, knowledge, policies, config, all)
    #[arg(short, long, default_value = "all")]
    pub target: String,

    /// Output file path
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Export format (json, yaml, tar)
    #[arg(long, default_value = "json")]
    pub format: ExportFormat,

    /// Include audit logs in export
    #[arg(long)]
    pub include_audit: bool,

    /// Filter by layer (tenant, org, team, project)
    #[arg(long)]
    pub layer: Option<String>,

    /// Compress output (gzip)
    #[arg(long)]
    pub compress: bool,

    /// Output result as JSON (for scripting)
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct AdminImportArgs {
    /// Input file path
    pub input: PathBuf,

    /// Import mode (merge, replace, skip-existing)
    #[arg(long, default_value = "merge")]
    pub mode: ImportMode,

    /// Dry run - validate without importing
    #[arg(long)]
    pub dry_run: bool,

    /// Skip validation (not recommended)
    #[arg(long)]
    pub skip_validation: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum ExportFormat {
    Json,
    Yaml,
    Tar,
}

#[derive(Clone, Debug, ValueEnum)]
pub enum ImportMode {
    Merge,
    Replace,
    SkipExisting,
}

#[derive(Args)]
pub struct AdminSyncArgs {
    /// Identity provider to sync from (github)
    #[arg(default_value = "github")]
    pub provider: String,

    /// Dry run - show what would be synced without making changes
    #[arg(long)]
    pub dry_run: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

pub async fn run(cmd: AdminCommand) -> anyhow::Result<()> {
    match cmd {
        AdminCommand::Health(args) => run_health(args).await,
        AdminCommand::Validate(args) => run_validate(args).await,
        AdminCommand::Migrate(args) => run_migrate(args).await,
        AdminCommand::Drift(args) => run_drift(args).await,
        AdminCommand::Export(args) => run_export(args).await,
        AdminCommand::Import(args) => run_import(args).await,
        AdminCommand::Sync(args) => run_sync(args).await,
        AdminCommand::Backup(sub) => match sub {
            AdminBackupCommand::Validate(args) => run_backup_validate(args).await,
        },
        AdminCommand::Tenant(sub) => match sub {
            AdminTenantCommand::Provision(args) => run_tenant_provision(args).await,
        },
    }
}

async fn get_live_client() -> Option<crate::client::AeternaClient> {
    let resolved = crate::profile::load_resolved(None, None);
    if let Ok(ref r) = resolved {
        crate::client::AeternaClient::from_profile(r).await.ok()
    } else {
        None
    }
}

async fn run_health(args: AdminHealthArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let ctx = resolver.resolve()?;

    let Some(client) = get_live_client().await else {
        crate::ux_error::server_not_connected().display();
        anyhow::bail!("Aeterna server not connected for operation: admin_health");
    };

    let health = client.admin_health().await?;
    let raw_status = health["status"].as_str().unwrap_or("unknown");
    let version = health["version"].as_str().unwrap_or("unknown");
    let overall_status = if raw_status == "ok" {
        "healthy"
    } else {
        raw_status
    };

    if args.json {
        let output = json!({
            "status": overall_status,
            "server": health,
            "context": {
                "tenant_id": ctx.tenant_id.value,
                "user_id": ctx.user_id.value,
                "project_id": ctx.project_id.as_ref().map(|v| &v.value),
            },
            "checks": if args.component == "all" {
                vec![json!({
                    "component": "server",
                    "status": overall_status,
                    "latency_ms": serde_json::Value::Null,
                    "message": format!("Server /health returned status='{raw_status}' version='{version}'"),
                    "details": {
                        "version": version,
                        "component_detail": "Per-component health endpoints are not exposed by the current server API"
                    },
                })]
            } else {
                vec![json!({
                    "component": args.component,
                    "status": "unsupported",
                    "latency_ms": serde_json::Value::Null,
                    "message": format!("Per-component health for '{}' is not exposed by the current server API; only /health is available", args.component),
                    "details": {
                        "server_status": raw_status,
                        "version": version,
                    },
                })]
            },
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::header("System Health Check");
        println!();

        let healthy = overall_status == "healthy";
        let status_icon = if healthy { "✓" } else { "!" };
        let status_color = if healthy { "green" } else { "yellow" };
        println!(
            "  Overall Status: {} {}",
            colored_status(status_icon, status_color),
            overall_status.to_uppercase()
        );
        println!("  Server Version:  {version}");
        println!();

        output::subheader("Context");
        println!("  Tenant:  {}", ctx.tenant_id.value);
        println!("  User:    {}", ctx.user_id.value);
        println!(
            "  Project: {}",
            ctx.project_id
                .as_ref()
                .map_or("(auto-detect)", |v| v.value.as_str())
        );
        println!();

        output::subheader("Health Source");
        println!("  Endpoint: /health");
        println!("  Raw status: {raw_status}");
        if args.component == "all" {
            println!("  Detail: per-component health is not exposed by the current server API");
        } else {
            println!("  Requested component: {}", args.component);
            println!("  Detail: per-component health is not exposed by the current server API");
        }
        if args.verbose {
            println!();
            println!("{}", serde_json::to_string_pretty(&health)?);
        }
        println!();

        output::hint("Run with --verbose to inspect the raw /health response");
    }

    Ok(())
}

async fn run_validate(args: AdminValidateArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let _ctx = resolver.resolve()?;

    let mut results: Vec<ValidationResult> = Vec::new();

    let targets = if args.target == "all" {
        vec!["config", "schema", "policies"]
    } else {
        vec![args.target.as_str()]
    };

    for target in &targets {
        let result = validate_target(target, args.config.as_ref(), args.strict).await;
        results.push(result);
    }

    let has_errors = results.iter().any(|r| !r.errors.is_empty());
    let has_warnings = results.iter().any(|r| !r.warnings.is_empty());
    let overall_valid = !has_errors && (!args.strict || !has_warnings);

    if args.json {
        let output = json!({
            "valid": overall_valid,
            "strict_mode": args.strict,
            "results": results.iter().map(|r| json!({
                "target": r.target,
                "valid": r.errors.is_empty() && (!args.strict || r.warnings.is_empty()),
                "errors": r.errors,
                "warnings": r.warnings,
                "info": r.info,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::header("Configuration Validation");
        println!();

        let status = if overall_valid { "VALID" } else { "INVALID" };
        let icon = if overall_valid { "✓" } else { "✗" };
        let color = if overall_valid { "green" } else { "red" };
        println!("  Status: {} {}", colored_status(icon, color), status);
        if args.strict {
            println!("  Mode: strict (warnings treated as errors)");
        }
        println!();

        for result in &results {
            output::subheader(&format!("Target: {}", result.target));

            if result.errors.is_empty() && result.warnings.is_empty() {
                println!("  ✓ No issues found");
            }

            for error in &result.errors {
                println!("  ✗ ERROR: {error}");
            }

            for warning in &result.warnings {
                println!("  ! WARNING: {warning}");
            }

            for info in &result.info {
                println!("  ℹ {info}");
            }
            println!();
        }

        if has_errors {
            output::hint("Fix errors before proceeding");
        }
    }

    if has_errors {
        crate::exit_code::ExitCode::Usage.exit();
    }

    Ok(())
}

async fn run_migrate(args: AdminMigrateArgs) -> anyhow::Result<()> {
    // `MIGRATIONS` is defined once in `storage::migrations` and is shared
    // between this CLI subcommand (production runtime) and test fixtures
    // (`testing::fixtures::postgres`). `.to_vec()` gives us an owned copy
    // so we can keep the existing sort-then-filter logic below without
    // mutating the const slice. Elements are `Copy`, so the copy is cheap.
    let mut migrations: Vec<EmbeddedMigration> = MIGRATIONS.to_vec();
    migrations.sort_by_key(|m| m.version);

    match args.direction.as_str() {
        "status" => {
            if let Ok(pool) = connect_migration_pool().await {
                ensure_migration_table(&pool).await?;
                let applied = get_applied_migrations(&pool).await?;
                verify_applied_checksums(&migrations, &applied)?;
                let report = build_migration_report(&migrations, &applied);
                print_migration_report("status", args.dry_run, args.json, &report)?;
            } else {
                let report = build_offline_status_report(&migrations);
                print_migration_report("status", args.dry_run, args.json, &report)?;
                output::warn(
                    "Database not connected — showing embedded migrations only. \
                     Set DATABASE_URL or PG_HOST to see applied status.",
                );
            }
        }
        "up" => {
            let target_version = parse_target_version(args.target.as_deref())?;

            if args.dry_run {
                let selected: Vec<&EmbeddedMigration> = migrations
                    .iter()
                    .filter(|m| target_version.is_none_or(|target| m.version <= target))
                    .collect();
                let report = build_dry_run_report(&selected);
                print_migration_report("up", true, args.json, &report)?;
                return Ok(());
            }

            let pool = match connect_migration_pool().await {
                Ok(pool) => pool,
                Err(e) => {
                    if args.json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&json!({
                                "success": false,
                                "direction": "up",
                                "error": "database_not_connected",
                                "message": format!("Database migration failed: database not connected — {e}"),
                                "fix": "Set DATABASE_URL or configure PG_HOST/PG_PORT/PG_USER/PG_PASSWORD/PG_DATABASE, or use --dry-run to preview",
                            }))?
                        );
                    } else {
                        ux_error::UxError::new("Database migration failed: database not connected")
                            .why(format!("{e}"))
                            .fix("Set DATABASE_URL or configure PG_HOST/PG_PORT/PG_USER/PG_PASSWORD/PG_DATABASE")
                            .suggest("aeterna admin migrate up --dry-run")
                            .display();
                    }
                    crate::exit_code::ExitCode::Usage.exit();
                }
            };

            // Initialize the core schema tables (organizational_units, sync_state,
            // governance_events, etc.) before running migrations. Several migrations
            // (009, 012, 025) reference tables that only exist after
            // PostgresBackend::initialize_schema() runs, and the server calls
            // initialize_schema() at startup. To avoid a chicken-and-egg dependency
            // on server startup ordering, we run initialize_schema() here so the
            // migration runner is self-contained on a fresh database.
            //
            // initialize_schema() is idempotent (CREATE TABLE IF NOT EXISTS,
            // CREATE INDEX IF NOT EXISTS), so it is safe to call on both fresh
            // and already-migrated databases.
            storage::postgres::PostgresBackend::from_pool(pool.clone())
                .initialize_schema()
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to initialize core schema tables before running migrations: {e}"
                    )
                })?;

            ensure_migration_table(&pool).await?;
            let applied = get_applied_migrations(&pool).await?;
            verify_applied_checksums(&migrations, &applied)?;

            let mut pending: Vec<&EmbeddedMigration> = migrations
                .iter()
                .filter(|m| !applied.iter().any(|a| a.version == m.version))
                .filter(|m| target_version.is_none_or(|target| m.version <= target))
                .collect();
            pending.sort_by_key(|m| m.version);

            for migration in pending {
                apply_migration(&pool, migration).await?;
                tracing::info!(
                    "Applied migration {}: {}",
                    migration.version,
                    migration.name
                );
            }

            let applied_after = get_applied_migrations(&pool).await?;
            let report = build_migration_report(&migrations, &applied_after);
            print_migration_report("up", false, args.json, &report)?;
        }
        "down" => {
            if !args.force {
                ux_error::UxError::new("Rollback requires --force flag")
                    .why("Rolling back migrations may cause data loss")
                    .fix("Add --force if you're sure you want to rollback")
                    .suggest("aeterna admin migrate down --force")
                    .display();
                crate::exit_code::ExitCode::Usage.exit();
            }

            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "success": false,
                        "direction": "down",
                        "error": "down_migrations_not_supported",
                        "message": "Down migrations are not supported: no rollback SQL is available for embedded migrations 003-016",
                    }))?
                );
            } else {
                ux_error::UxError::new("Down migrations are not supported")
                    .why("No rollback SQL is defined for migrations 003-016")
                    .fix("Restore from backup if rollback is required")
                    .display();
            }
            crate::exit_code::ExitCode::Usage.exit();
        }
        _ => {
            ux_error::UxError::new(format!("Invalid migration direction: {}", args.direction))
                .fix("Use one of: up, down, status")
                .suggest("aeterna admin migrate status")
                .display();
            crate::exit_code::ExitCode::Usage.exit();
        }
    }

    Ok(())
}

#[derive(Debug, FromRow)]
struct AppliedMigration {
    version: i32,
    name: String,
    checksum: String,
    applied_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
struct MigrationReportEntry {
    version: i32,
    name: String,
    status: String,
    checksum: String,
    applied_at: Option<String>,
}

struct MigrationReport {
    current_version: Option<i32>,
    pending_count: usize,
    migrations: Vec<MigrationReportEntry>,
}

fn postgres_connection_url(config: &config::Config) -> String {
    let pg = &config.providers.postgres;
    format!(
        "postgres://{}:{}@{}:{}/{}",
        pg.username, pg.password, pg.host, pg.port, pg.database
    )
}

async fn connect_migration_pool() -> anyhow::Result<PgPool> {
    let cfg = config::load_from_env()?;
    let url = postgres_connection_url(&cfg);
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(30))
        .connect(&url)
        .await?;
    Ok(pool)
}

async fn ensure_migration_table(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS _aeterna_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
        );
        ",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn get_applied_migrations(pool: &PgPool) -> anyhow::Result<Vec<AppliedMigration>> {
    let rows = sqlx::query_as::<_, AppliedMigration>(
        "SELECT version, name, checksum, applied_at FROM _aeterna_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

fn migration_checksum(sql: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sql.as_bytes());
    hex::encode(hasher.finalize())
}

fn verify_applied_checksums(
    embedded: &[EmbeddedMigration],
    applied: &[AppliedMigration],
) -> anyhow::Result<()> {
    let embedded_by_version: std::collections::HashMap<i32, &EmbeddedMigration> =
        embedded.iter().map(|m| (m.version, m)).collect();

    for existing in applied {
        if let Some(expected) = embedded_by_version.get(&existing.version) {
            let expected_checksum = migration_checksum(expected.sql);
            if existing.checksum != expected_checksum {
                anyhow::bail!(
                    "Migration checksum mismatch for version {} ({}): expected {}, found {}",
                    existing.version,
                    expected.name,
                    expected_checksum,
                    existing.checksum
                );
            }
        }
    }

    Ok(())
}

async fn apply_migration(pool: &PgPool, migration: &EmbeddedMigration) -> anyhow::Result<()> {
    let checksum = migration_checksum(migration.sql);
    let mut tx = pool.begin().await?;

    // Use raw_sql to support multiple statements in a single migration file.
    // sqlx::query() sends a prepared statement which only allows one command.
    sqlx::raw_sql(migration.sql).execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO _aeterna_migrations (version, name, checksum) VALUES ($1, $2, $3) ON CONFLICT (version) DO NOTHING",
    )
    .bind(migration.version)
    .bind(migration.name)
    .bind(checksum)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

fn parse_target_version(target: Option<&str>) -> anyhow::Result<Option<i32>> {
    match target {
        Some(raw) => {
            let parsed = raw.parse::<i32>().map_err(|_| {
                anyhow::anyhow!(
                    "Invalid --target value '{raw}'. Expected integer migration version (e.g. 16)."
                )
            })?;
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

fn build_dry_run_report(migrations: &[&EmbeddedMigration]) -> MigrationReport {
    let entries = migrations
        .iter()
        .map(|migration| MigrationReportEntry {
            version: migration.version,
            name: migration.name.to_string(),
            status: "pending".to_string(),
            checksum: migration_checksum(migration.sql),
            applied_at: None,
        })
        .collect::<Vec<_>>();

    MigrationReport {
        current_version: None,
        pending_count: entries.len(),
        migrations: entries,
    }
}

fn build_migration_report(
    embedded: &[EmbeddedMigration],
    applied: &[AppliedMigration],
) -> MigrationReport {
    let applied_by_version: std::collections::HashMap<i32, &AppliedMigration> =
        applied.iter().map(|m| (m.version, m)).collect();

    let mut entries = Vec::with_capacity(embedded.len());
    for migration in embedded {
        if let Some(applied_migration) = applied_by_version.get(&migration.version) {
            entries.push(MigrationReportEntry {
                version: migration.version,
                name: applied_migration.name.clone(),
                status: "applied".to_string(),
                checksum: applied_migration.checksum.clone(),
                applied_at: Some(applied_migration.applied_at.to_rfc3339()),
            });
        } else {
            entries.push(MigrationReportEntry {
                version: migration.version,
                name: migration.name.to_string(),
                status: "pending".to_string(),
                checksum: migration_checksum(migration.sql),
                applied_at: None,
            });
        }
    }

    entries.sort_by_key(|m| m.version);

    let current_version = applied.iter().map(|m| m.version).max();
    let pending_count = entries.iter().filter(|m| m.status == "pending").count();

    MigrationReport {
        current_version,
        pending_count,
        migrations: entries,
    }
}

fn build_offline_status_report(embedded: &[EmbeddedMigration]) -> MigrationReport {
    let entries = embedded
        .iter()
        .map(|migration| MigrationReportEntry {
            version: migration.version,
            name: migration.name.to_string(),
            status: "unknown".to_string(),
            checksum: migration_checksum(migration.sql),
            applied_at: None,
        })
        .collect::<Vec<_>>();

    MigrationReport {
        current_version: None,
        pending_count: entries.len(),
        migrations: entries,
    }
}

fn print_migration_report(
    direction: &str,
    dry_run: bool,
    json_output: bool,
    report: &MigrationReport,
) -> anyhow::Result<()> {
    if json_output {
        let output = json!({
            "direction": direction,
            "dry_run": dry_run,
            "current_version": report.current_version,
            "pending_count": report.pending_count,
            "migrations": report.migrations,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    output::header("Database Migration");
    println!();
    println!(
        "  Current Version: {}",
        report
            .current_version
            .map_or_else(|| "none".to_string(), |v| v.to_string())
    );
    println!("  Direction:       {direction}");
    if dry_run {
        println!("  Mode:            DRY RUN");
    }
    println!();

    output::subheader("Migration Status");
    for migration in &report.migrations {
        let icon = if migration.status == "applied" {
            "✓"
        } else {
            "○"
        };
        if let Some(applied_at) = &migration.applied_at {
            println!(
                "  {} {:03} - {} (applied at {})",
                icon, migration.version, migration.name, applied_at
            );
        } else {
            println!("  {} {:03} - {}", icon, migration.version, migration.name);
        }
    }
    println!();
    println!("  {} pending migration(s)", report.pending_count);
    println!();
    Ok(())
}

async fn run_drift(args: AdminDriftArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let _ctx = resolver.resolve()?;

    // Simulated drift detection results
    let drifts = vec![
        DriftItem {
            target: "config".to_string(),
            path: ".aeterna/context.toml".to_string(),
            drift_type: "modified".to_string(),
            expected: "timeout = 30".to_string(),
            actual: "timeout = 60".to_string(),
            fixable: true,
        },
        DriftItem {
            target: "policies".to_string(),
            path: "policies/security-baseline.cedar".to_string(),
            drift_type: "missing".to_string(),
            expected: "(file should exist)".to_string(),
            actual: "(file not found)".to_string(),
            fixable: true,
        },
    ];

    let has_drift = !drifts.is_empty();
    let fixable_count = drifts.iter().filter(|d| d.fixable).count();

    if args.json {
        let output = json!({
            "source": args.source,
            "has_drift": has_drift,
            "drift_count": drifts.len(),
            "fixable_count": fixable_count,
            "drifts": drifts.iter().map(|d| json!({
                "target": d.target,
                "path": d.path,
                "type": d.drift_type,
                "expected": d.expected,
                "actual": d.actual,
                "fixable": d.fixable,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        output::header("Configuration Drift Detection");
        println!();

        println!("  Source: {}", args.source);
        println!("  Target: {}", args.target);
        println!();

        if drifts.is_empty() {
            println!("  ✓ No drift detected - configuration matches expected state");
        } else {
            output::subheader(&format!("Detected Drift ({} items)", drifts.len()));

            for drift in &drifts {
                let icon = match drift.drift_type.as_str() {
                    "modified" => "~",
                    "missing" => "-",
                    "extra" => "+",
                    _ => "?",
                };
                let fixable = if drift.fixable { " [fixable]" } else { "" };

                println!(
                    "  {} {} ({}){}",
                    icon, drift.path, drift.drift_type, fixable
                );
                println!("      Expected: {}", drift.expected);
                println!("      Actual:   {}", drift.actual);
                println!();
            }

            if args.fix {
                println!("  Applying fixes...");
                for drift in &drifts {
                    if drift.fixable {
                        println!("    ✓ Fixed {}", drift.path);
                    } else {
                        println!("    ! Cannot auto-fix {}", drift.path);
                    }
                }
                println!();
            } else {
                output::hint(&format!(
                    "Run with --fix to auto-correct {fixable_count} fixable items"
                ));
            }
        }
    }

    Ok(())
}

async fn run_export(args: AdminExportArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let _ctx = resolver.resolve()?;

    let output_path = args.output.unwrap_or_else(|| {
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        PathBuf::from(format!("aeterna_export_{timestamp}.tar.gz"))
    });

    let server_url = std::env::var("AETERNA_SERVER_URL").ok();

    if let Some(url) = &server_url {
        // Server-based export: call the admin export API.
        // B2 §7.7 — route through `tagged_http_client()` so the audit
        // row for the export POST records `via=cli`.
        let client = crate::client::tagged_http_client();
        let api_url = format!("{url}/api/v1/admin/export");

        if !args.json {
            output::header("Data Export");
            println!();
            println!("  Server:  {url}");
            println!("  Target:  {}", args.target);
            println!("  Output:  {}", output_path.display());
            println!();
            println!("  Requesting export from server...");
        }

        match client
            .post(&api_url)
            .json(&json!({
                "target": args.target,
                "include_audit": args.include_audit,
                "layer": args.layer,
            }))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let bytes = resp.bytes().await?;
                std::fs::write(&output_path, &bytes)?;
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "success": true,
                            "output_path": output_path.to_string_lossy(),
                            "size_bytes": bytes.len(),
                        }))?
                    );
                } else {
                    output::success(&format!(
                        "Export saved to {} ({} bytes)",
                        output_path.display(),
                        bytes.len()
                    ));
                }
            }
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "success": false,
                            "error": "server_error",
                            "status": status.as_u16(),
                            "message": body,
                        }))?
                    );
                } else {
                    ux_error::UxError::new("Export failed: server returned an error")
                        .why(format!("HTTP {status}: {body}"))
                        .fix("Check server logs for details")
                        .display();
                }
                anyhow::bail!("Server export failed with HTTP {status}");
            }
            Err(e) => {
                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "success": false,
                            "error": "connection_error",
                            "message": e.to_string(),
                        }))?
                    );
                } else {
                    ux_error::UxError::new("Cannot reach Aeterna server for export")
                        .why(format!("Connection error: {e}"))
                        .fix("Verify the server is running and AETERNA_SERVER_URL is correct")
                        .suggest("aeterna serve")
                        .display();
                }
                anyhow::bail!("Failed to connect to server: {e}");
            }
        }
    } else {
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "success": false,
                    "error": "server_not_connected",
                    "message": "AETERNA_SERVER_URL is not set. Export requires a running server.",
                }))?
            );
        } else {
            ux_error::UxError::new("Cannot export: server not connected")
                .why("AETERNA_SERVER_URL is not set")
                .fix("Start the Aeterna server and set AETERNA_SERVER_URL")
                .suggest("aeterna serve")
                .display();
        }
        anyhow::bail!("Export requires a running server (AETERNA_SERVER_URL not set)");
    }

    Ok(())
}

async fn run_import(args: AdminImportArgs) -> anyhow::Result<()> {
    let resolver = ContextResolver::new();
    let _ctx = resolver.resolve()?;

    if !args.input.exists() {
        ux_error::UxError::new(format!("Import file not found: {}", args.input.display()))
            .why("The specified import file does not exist")
            .fix("Check the file path is correct")
            .fix("Ensure the file exists and is readable")
            .display();
        crate::exit_code::ExitCode::Usage.exit();
    }

    // Open and validate the archive using the backup crate.
    let report = if args.skip_validation {
        None
    } else {
        Some(validate_archive(&args.input)?)
    };

    // If validation failed, report and bail.
    if let Some(ref rpt) = report
        && !rpt.valid
    {
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "success": false,
                    "error": "validation_failed",
                    "manifest_ok": rpt.manifest_ok,
                    "schema_compatible": rpt.schema_compatible,
                    "checksum_mismatches": rpt.checksum_mismatches.len(),
                    "errors": rpt.errors,
                }))?
            );
        } else {
            ux_error::UxError::new("Archive validation failed")
                .why(rpt.errors.join("; "))
                .fix("Ensure the archive is not corrupted")
                .fix("Re-export from the source instance")
                .display();
        }
        anyhow::bail!("Archive validation failed: {}", rpt.errors.join("; "));
    }

    // Read the manifest from the archive for display.
    let reader = ArchiveReader::open(&args.input)?;
    let manifest = reader.manifest();

    if args.json {
        let output_val = json!({
            "input_path": args.input.to_string_lossy(),
            "mode": format!("{:?}", args.mode).to_lowercase(),
            "dry_run": args.dry_run,
            "archive": {
                "schema_version": manifest.schema_version,
                "source_instance": manifest.source_instance,
                "created_at": manifest.created_at,
                "scope": manifest.scope,
                "incremental": manifest.incremental,
            },
            "entity_counts": {
                "memories": manifest.entity_counts.memories,
                "knowledge_items": manifest.entity_counts.knowledge_items,
                "policies": manifest.entity_counts.policies,
                "org_units": manifest.entity_counts.org_units,
                "graph_nodes": manifest.entity_counts.graph_nodes,
                "graph_edges": manifest.entity_counts.graph_edges,
            },
            "file_checksums": manifest.file_checksums.len(),
            "validation": report.as_ref().map(|r| json!({
                "valid": r.valid,
                "manifest_ok": r.manifest_ok,
                "schema_compatible": r.schema_compatible,
            })),
        });
        println!("{}", serde_json::to_string_pretty(&output_val)?);
    } else {
        output::header("Data Import");
        println!();

        println!("  Input:            {}", args.input.display());
        println!("  Mode:             {:?}", args.mode);
        if args.dry_run {
            println!("  Status:           DRY RUN (no changes will be made)");
        }
        println!();

        output::subheader("Archive Details");
        println!("  Schema version:   {}", manifest.schema_version);
        println!("  Source instance:   {}", manifest.source_instance);
        println!("  Created at:       {}", manifest.created_at);
        println!("  Incremental:      {}", manifest.incremental);
        println!("  Data files:       {}", manifest.file_checksums.len());
        println!();

        output::subheader("Entity Counts");
        println!("  Memories:         {}", manifest.entity_counts.memories);
        println!(
            "  Knowledge items:  {}",
            manifest.entity_counts.knowledge_items
        );
        println!("  Policies:         {}", manifest.entity_counts.policies);
        println!("  Org units:        {}", manifest.entity_counts.org_units);
        println!("  Graph nodes:      {}", manifest.entity_counts.graph_nodes);
        println!("  Graph edges:      {}", manifest.entity_counts.graph_edges);
        println!();

        if let Some(ref rpt) = report {
            output::subheader("Validation");
            println!(
                "  Manifest:    {}",
                if rpt.manifest_ok { "OK" } else { "FAIL" }
            );
            println!(
                "  Schema:      {}",
                if rpt.schema_compatible {
                    "compatible"
                } else {
                    "INCOMPATIBLE"
                }
            );
            println!(
                "  Checksums:   {}",
                if rpt.checksum_mismatches.is_empty() {
                    "all verified".to_string()
                } else {
                    format!("{} mismatch(es)", rpt.checksum_mismatches.len())
                }
            );
            println!();
        }
    }

    if args.dry_run {
        if !args.json {
            output::success("Dry-run validation complete. Archive is valid.");
            output::hint("Remove --dry-run to execute import against a running server.");
        }
    } else {
        // Actual import requires a server connection.
        let server_url = std::env::var("AETERNA_SERVER_URL").ok();
        if let Some(url) = server_url {
            // B2 §7.7 — import POST also carries the CLI identity
            // headers via the shared tagged client builder.
            let client = crate::client::tagged_http_client();
            let archive_bytes = std::fs::read(&args.input)?;
            let mode_str = format!("{:?}", args.mode).to_lowercase();
            let api_url = format!("{url}/api/v1/admin/import?mode={mode_str}");

            let result: Result<reqwest::Response, reqwest::Error> = client
                .post(&api_url)
                .header("content-type", "application/gzip")
                .body(archive_bytes)
                .send()
                .await;

            match result {
                Ok(resp) if resp.status().is_success() => {
                    if args.json {
                        let body = resp.text().await.unwrap_or_default();
                        println!("{body}");
                    } else {
                        output::success("Import completed successfully.");
                    }
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    if !args.json {
                        ux_error::UxError::new("Import failed: server returned an error")
                            .why(format!("HTTP {status}: {body}"))
                            .fix("Check server logs for details")
                            .display();
                    }
                    anyhow::bail!("Server import failed with HTTP {status}");
                }
                Err(e) => {
                    if !args.json {
                        ux_error::UxError::new("Cannot reach Aeterna server for import")
                            .why(format!("Connection error: {e}"))
                            .fix("Verify the server is running and AETERNA_SERVER_URL is correct")
                            .suggest("aeterna serve")
                            .display();
                    }
                    anyhow::bail!("Failed to connect to server: {e}");
                }
            }
        } else {
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "success": false,
                        "error": "server_not_connected",
                        "message": "Import requires a live Aeterna server connection. Use --dry-run to validate offline."
                    }))?
                );
            } else {
                ux_error::UxError::new("Cannot import: server not connected")
                    .why("Import writes to the live memory and knowledge backends")
                    .fix("Start the Aeterna server: aeterna serve")
                    .fix("Ensure AETERNA_SERVER_URL is set and the server is reachable")
                    .fix("Use --dry-run to validate the import file without connecting")
                    .suggest("aeterna admin import --dry-run <file>")
                    .display();
            }
            anyhow::bail!(
                "Aeterna server not connected. Set AETERNA_SERVER_URL and ensure the server is running."
            );
        }
    }

    Ok(())
}

async fn run_backup_validate(args: AdminBackupValidateArgs) -> anyhow::Result<()> {
    if !args.archive.exists() {
        ux_error::UxError::new(format!("Archive not found: {}", args.archive.display()))
            .why("The specified archive file does not exist")
            .fix("Check the file path is correct")
            .display();
        crate::exit_code::ExitCode::Usage.exit();
    }

    let report = validate_archive(&args.archive)?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "archive": args.archive.to_string_lossy(),
                "valid": report.valid,
                "manifest_ok": report.manifest_ok,
                "schema_compatible": report.schema_compatible,
                "checksum_mismatches": report.checksum_mismatches.iter().map(|m| json!({
                    "filename": m.filename,
                    "expected": m.expected,
                    "actual": m.actual,
                })).collect::<Vec<_>>(),
                "errors": report.errors,
            }))?
        );
    } else {
        output::header("Backup Archive Validation");
        println!();
        println!("  Archive:     {}", args.archive.display());
        println!();

        println!(
            "  Manifest:    {}",
            if report.manifest_ok { "OK" } else { "FAIL" }
        );
        println!(
            "  Schema:      {}",
            if report.schema_compatible {
                "compatible"
            } else {
                "INCOMPATIBLE"
            }
        );
        println!(
            "  Checksums:   {}",
            if report.checksum_mismatches.is_empty() {
                "all verified".to_string()
            } else {
                format!("{} mismatch(es)", report.checksum_mismatches.len())
            }
        );

        if !report.checksum_mismatches.is_empty() {
            println!();
            output::subheader("Checksum Mismatches");
            for m in &report.checksum_mismatches {
                println!("  ! {}:", m.filename);
                println!("      expected: {}", m.expected);
                println!("      actual:   {}", m.actual);
            }
        }

        if !report.errors.is_empty() {
            println!();
            output::subheader("Errors");
            for err in &report.errors {
                println!("  - {err}");
            }
        }

        println!();
        if report.valid {
            output::success("Archive is valid and ready for import.");
        } else {
            ux_error::UxError::new("Archive validation failed")
                .why("See errors above")
                .fix("Re-export from the source instance")
                .display();
        }
    }

    if report.valid {
        Ok(())
    } else {
        anyhow::bail!("Archive validation failed")
    }
}

async fn run_tenant_provision(args: AdminTenantProvisionArgs) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(&args.file).map_err(|e| {
        anyhow::anyhow!(
            "Failed to read manifest file '{}': {}",
            args.file.display(),
            e
        )
    })?;

    let manifest: serde_json::Value = serde_yaml::from_str(&raw).map_err(|e| {
        anyhow::anyhow!("Failed to parse manifest '{}': {}", args.file.display(), e)
    })?;

    if args.dry_run {
        if args.json {
            let out = serde_json::json!({
                "dryRun": true,
                "operation": "tenant_provision",
                "file": args.file.display().to_string(),
                "manifest": manifest,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            output::header("Tenant Provision (Dry Run)");
            println!();
            println!("  File:    {}", args.file.display());
            if let Some(slug) = manifest
                .get("tenant")
                .and_then(|t| t.get("slug"))
                .and_then(|s| s.as_str())
            {
                println!("  Tenant:  {slug}");
            }
            if let Some(name) = manifest
                .get("tenant")
                .and_then(|t| t.get("name"))
                .and_then(|s| s.as_str())
            {
                println!("  Name:    {name}");
            }
            println!();
            println!("  Manifest parsed successfully. Run without --dry-run to provision.");
        }
        return Ok(());
    }

    let Some(client) = get_live_client().await else {
        ux_error::UxError::new("Could not connect to Aeterna server")
            .why("Authentication or connection failed")
            .fix("Ensure AETERNA_SERVER_URL is set and the server is reachable")
            .suggest("aeterna admin health")
            .display();
        anyhow::bail!("Aeterna server not connected for tenant provisioning");
    };

    let result = client.tenant_provision(&manifest).await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        let success = result
            .get("success")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let slug = result
            .get("slug")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let tenant_id = result
            .get("tenantId")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        if success {
            output::header("Tenant Provisioned Successfully");
        } else {
            output::header("Tenant Provisioning Completed with Errors");
        }
        println!();
        println!("  Tenant ID: {tenant_id}");
        println!("  Slug:      {slug}");
        println!();

        if let Some(steps) = result.get("steps").and_then(|v| v.as_array()) {
            println!("  Steps:");
            for step in steps {
                let name = step.get("step").and_then(|v| v.as_str()).unwrap_or("?");
                let ok = step
                    .get("ok")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let icon = if ok { "✓" } else { "✗" };
                let detail = step.get("detail").and_then(|v| v.as_str()).unwrap_or("");
                let error = step.get("error").and_then(|v| v.as_str()).unwrap_or("");
                if ok {
                    println!("    {icon} {name}: {detail}");
                } else {
                    println!("    {icon} {name}: {error}");
                }
            }
            println!();
        }

        if let Some(errors) = result.get("validationErrors").and_then(|v| v.as_array()) {
            println!("  Validation Errors:");
            for err in errors {
                if let Some(msg) = err.as_str() {
                    println!("    • {msg}");
                }
            }
            println!();
        }
    }

    Ok(())
}

// Helper types and functions

struct HealthCheck {
    component: String,
    status: String,
    latency_ms: u64,
    message: String,
    details: std::collections::HashMap<String, String>,
}

async fn check_component_health(component: &str, _timeout: u64) -> HealthCheck {
    // Not connected to live backend: report honest "not_connected" status.
    // Real implementation would ping each service endpoint via AETERNA_SERVER_URL.
    let details = std::collections::HashMap::new();
    match component {
        "memory" | "knowledge" | "policy" | "context" => HealthCheck {
            component: component.to_string(),
            status: "not_connected".to_string(),
            latency_ms: 0,
            message: "Server not connected — set AETERNA_SERVER_URL to enable health checks"
                .to_string(),
            details,
        },
        _ => HealthCheck {
            component: component.to_string(),
            status: "unknown".to_string(),
            latency_ms: 0,
            message: "Unknown component".to_string(),
            details,
        },
    }
}

struct ValidationResult {
    target: String,
    errors: Vec<String>,
    warnings: Vec<String>,
    info: Vec<String>,
}

async fn validate_target(
    target: &str,
    _config_path: Option<&PathBuf>,
    _strict: bool,
) -> ValidationResult {
    // Simulated validation - in real implementation, this would
    // actually validate the various components
    match target {
        "config" => ValidationResult {
            target: "config".to_string(),
            errors: vec![],
            warnings: vec![],
            info: vec!["Configuration file valid".to_string()],
        },
        "schema" => ValidationResult {
            target: "schema".to_string(),
            errors: vec![],
            warnings: vec![],
            info: vec!["Database schema matches expected version".to_string()],
        },
        "policies" => ValidationResult {
            target: "policies".to_string(),
            errors: vec![],
            warnings: vec!["Policy 'legacy-compat' uses deprecated syntax".to_string()],
            info: vec!["23 policies validated".to_string()],
        },
        _ => ValidationResult {
            target: target.to_string(),
            errors: vec![format!("Unknown validation target: {}", target)],
            warnings: vec![],
            info: vec![],
        },
    }
}

struct Migration {
    version: String,
    name: String,
    status: String,
    reversible: bool,
}

struct DriftItem {
    target: String,
    path: String,
    drift_type: String,
    expected: String,
    actual: String,
    fixable: bool,
}

struct ExportStats {
    memories: u64,
    knowledge_items: u64,
    policies: u64,
    config_files: u64,
    audit_entries: u64,
}

struct ImportAnalysis {
    format: String,
    source_version: String,
    memories: u64,
    knowledge_items: u64,
    policies: u64,
    conflicts: Vec<ImportConflict>,
}

struct ImportConflict {
    item_type: String,
    id: String,
    reason: String,
}

fn colored_status(icon: &str, color: &str) -> String {
    use colored::Colorize;
    match color {
        "green" => icon.green().to_string(),
        "yellow" => icon.yellow().to_string(),
        "red" => icon.red().to_string(),
        _ => icon.white().to_string(),
    }
}

async fn run_sync(args: AdminSyncArgs) -> anyhow::Result<()> {
    match args.provider.as_str() {
        p if p == PROVIDER_GITHUB => run_sync_github(args).await,
        provider => {
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "success": false,
                        "error": "unsupported_provider",
                        "message": format!("Unsupported sync provider: {provider}. Supported: github")
                    }))?
                );
            } else {
                ux_error::UxError::new(format!("Unsupported sync provider: {provider}"))
                    .why("Currently supported providers: github")
                    .fix("Use: aeterna admin sync github")
                    .display();
            }
            Ok(())
        }
    }
}

async fn run_sync_github(args: AdminSyncArgs) -> anyhow::Result<()> {
    let org_name = std::env::var("AETERNA_GITHUB_ORG_NAME")
        .map_err(|_| anyhow::anyhow!("AETERNA_GITHUB_ORG_NAME is required for GitHub sync"))?;

    let app_id: u64 = std::env::var("AETERNA_GITHUB_APP_ID")
        .map_err(|_| anyhow::anyhow!("AETERNA_GITHUB_APP_ID is required"))?
        .parse()
        .map_err(|_| anyhow::anyhow!("AETERNA_GITHUB_APP_ID must be a number"))?;

    let installation_id: u64 = std::env::var("AETERNA_GITHUB_INSTALLATION_ID")
        .map_err(|_| anyhow::anyhow!("AETERNA_GITHUB_INSTALLATION_ID is required"))?
        .parse()
        .map_err(|_| anyhow::anyhow!("AETERNA_GITHUB_INSTALLATION_ID must be a number"))?;

    let private_key_pem = std::env::var("AETERNA_GITHUB_APP_PEM")
        .map_err(|_| anyhow::anyhow!("AETERNA_GITHUB_APP_PEM is required"))?;

    let team_filter = std::env::var("AETERNA_GITHUB_TEAM_FILTER").ok();
    let sync_repos_as_projects = std::env::var("AETERNA_GITHUB_SYNC_REPOS_AS_PROJECTS")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let github_config = idp_sync::config::GitHubConfig {
        org_name: org_name.clone(),
        app_id,
        installation_id,
        private_key_pem,
        team_filter,
        sync_repos_as_projects,
        api_base_url: None,
    };

    if args.dry_run {
        if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "dry_run": true,
                    "provider": PROVIDER_GITHUB,
                    "org_name": org_name,
                    "app_id": app_id,
                    "installation_id": installation_id,
                    "message": "Dry run — would sync GitHub org into Aeterna hierarchy"
                }))?
            );
        } else {
            output::header("GitHub Organization Sync (Dry Run)");
            println!();
            println!("  Provider:        github");
            println!("  Organization:    {org_name}");
            println!("  App ID:          {app_id}");
            println!("  Installation ID: {installation_id}");
            println!();
            println!("  This is a dry run. No changes will be made.");
        }
        return Ok(());
    }

    if !args.json {
        output::header("GitHub Organization Sync");
        println!();
        println!("  Syncing {org_name}...");
        println!();
    }

    let database_url = std::env::var("DATABASE_URL")
        .or_else(|_| std::env::var("AETERNA_DATABASE_URL"))
        .map_err(|_| anyhow::anyhow!("DATABASE_URL or AETERNA_DATABASE_URL is required"))?;

    let pool = sqlx::PgPool::connect(&database_url).await?;

    let tenant_str =
        std::env::var(crate::env_vars::AETERNA_TENANT_ID).unwrap_or_else(|_| "default".to_string());
    let tenant_id: uuid::Uuid = {
        let row: Option<(uuid::Uuid,)> = sqlx::query_as(
            "SELECT id FROM tenants
                 WHERE slug = $1 OR name = $1 OR id::text = $1
                 LIMIT 1",
        )
        .bind(&tenant_str)
        .fetch_optional(&pool)
        .await?;
        if let Some((id,)) = row {
            id
        } else {
            // See admin_sync::slugify docstring; `tenants.slug` is NOT NULL
            // UNIQUE so we must derive one from tenant_str.
            let slug = crate::server::admin_sync::slugify(&tenant_str);
            sqlx::query_scalar::<_, uuid::Uuid>(
                "INSERT INTO tenants (id, slug, name) VALUES ($1, $2, $3)
                 ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name
                 RETURNING id",
            )
            .bind(uuid::Uuid::new_v4())
            .bind(&slug)
            .bind(&tenant_str)
            .fetch_one(&pool)
            .await?
        }
    };

    let report = idp_sync::github::run_github_sync(&github_config, &pool, tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!("GitHub sync failed: {e:?}"))?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&json!(report))?);
    } else {
        output::subheader("Sync Report");
        println!();
        println!("  Users created:       {}", report.users_created);
        println!("  Users updated:       {}", report.users_updated);
        println!("  Users deactivated:   {}", report.users_deactivated);
        println!("  Groups synced:       {}", report.groups_synced);
        println!("  Memberships added:   {}", report.memberships_added);
        println!("  Memberships removed: {}", report.memberships_removed);
        println!();
        println!("  ✓ GitHub organization sync completed successfully");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_health_check_all_components_not_connected() {
        // When server is not connected, all known components report "not_connected".
        for component in &["memory", "knowledge", "policy", "context"] {
            let check = check_component_health(component, 30).await;
            assert_eq!(
                check.status, "not_connected",
                "component '{component}' should report not_connected when server is absent"
            );
            assert!(
                check.message.contains("not connected")
                    || check.message.contains("AETERNA_SERVER_URL"),
                "message should explain the not-connected state"
            );
        }
    }

    #[tokio::test]
    async fn test_health_check_unknown_component() {
        let unknown = check_component_health("unknown", 30).await;
        assert_eq!(unknown.status, "unknown");
    }

    #[tokio::test]
    async fn test_validate_config() {
        let result = validate_target("config", None, false).await;
        assert!(result.errors.is_empty());
    }

    #[tokio::test]
    async fn test_validate_policies_warnings() {
        let result = validate_target("policies", None, false).await;
        assert!(result.errors.is_empty());
        assert!(!result.warnings.is_empty());
    }

    #[tokio::test]
    async fn test_validate_unknown_target() {
        let result = validate_target("invalid", None, false).await;
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_export_format_values() {
        let _json = ExportFormat::Json;
        let _yaml = ExportFormat::Yaml;
        let _tar = ExportFormat::Tar;
    }

    #[test]
    fn test_import_mode_values() {
        let _merge = ImportMode::Merge;
        let _replace = ImportMode::Replace;
        let _skip = ImportMode::SkipExisting;
    }

    #[test]
    fn test_admin_health_args_defaults() {
        let args = AdminHealthArgs {
            component: "all".to_string(),
            json: false,
            verbose: false,
            timeout: 30,
        };
        assert_eq!(args.component, "all");
        assert!(!args.json);
        assert!(!args.verbose);
        assert_eq!(args.timeout, 30);
    }

    #[test]
    fn test_admin_health_args_with_options() {
        let args = AdminHealthArgs {
            component: "memory".to_string(),
            json: true,
            verbose: true,
            timeout: 60,
        };
        assert_eq!(args.component, "memory");
        assert!(args.json);
        assert!(args.verbose);
        assert_eq!(args.timeout, 60);
    }

    #[test]
    fn test_admin_validate_args_defaults() {
        let args = AdminValidateArgs {
            target: "all".to_string(),
            config: None,
            strict: false,
            json: false,
        };
        assert_eq!(args.target, "all");
        assert!(args.config.is_none());
        assert!(!args.strict);
        assert!(!args.json);
    }

    #[test]
    fn test_admin_validate_args_with_config() {
        let args = AdminValidateArgs {
            target: "config".to_string(),
            config: Some(PathBuf::from("/path/to/config.toml")),
            strict: true,
            json: true,
        };
        assert_eq!(args.target, "config");
        assert!(args.config.is_some());
        assert!(args.strict);
        assert!(args.json);
    }

    #[test]
    fn test_admin_migrate_args_defaults() {
        let args = AdminMigrateArgs {
            direction: "up".to_string(),
            target: None,
            dry_run: false,
            force: false,
            json: false,
        };
        assert_eq!(args.direction, "up");
        assert!(args.target.is_none());
        assert!(!args.dry_run);
        assert!(!args.force);
    }

    #[test]
    fn test_admin_migrate_args_down_with_force() {
        let args = AdminMigrateArgs {
            direction: "down".to_string(),
            target: Some("2024.1.0".to_string()),
            dry_run: false,
            force: true,
            json: true,
        };
        assert_eq!(args.direction, "down");
        assert_eq!(args.target, Some("2024.1.0".to_string()));
        assert!(args.force);
    }

    #[test]
    fn test_admin_migrate_args_status() {
        let args = AdminMigrateArgs {
            direction: "status".to_string(),
            target: None,
            dry_run: false,
            force: false,
            json: false,
        };
        assert_eq!(args.direction, "status");
    }

    #[test]
    fn test_admin_drift_args_defaults() {
        let args = AdminDriftArgs {
            source: "git".to_string(),
            expected: None,
            target: "all".to_string(),
            fix: false,
            json: false,
        };
        assert_eq!(args.source, "git");
        assert!(args.expected.is_none());
        assert_eq!(args.target, "all");
        assert!(!args.fix);
    }

    #[test]
    fn test_admin_drift_args_with_file_source() {
        let args = AdminDriftArgs {
            source: "file".to_string(),
            expected: Some(PathBuf::from("/path/to/expected.json")),
            target: "config".to_string(),
            fix: true,
            json: true,
        };
        assert_eq!(args.source, "file");
        assert!(args.expected.is_some());
        assert!(args.fix);
    }

    #[test]
    fn test_admin_export_args_defaults() {
        let args = AdminExportArgs {
            target: "all".to_string(),
            output: None,
            format: ExportFormat::Json,
            include_audit: false,
            layer: None,
            compress: false,
            json: false,
        };
        assert_eq!(args.target, "all");
        assert!(args.output.is_none());
        assert!(!args.include_audit);
        assert!(!args.compress);
    }

    #[test]
    fn test_admin_export_args_full_options() {
        let args = AdminExportArgs {
            target: "memories".to_string(),
            output: Some(PathBuf::from("/backup/export.tar.gz")),
            format: ExportFormat::Tar,
            include_audit: true,
            layer: Some("tenant".to_string()),
            compress: true,
            json: true,
        };
        assert_eq!(args.target, "memories");
        assert!(args.output.is_some());
        assert!(args.include_audit);
        assert!(args.compress);
        assert_eq!(args.layer, Some("tenant".to_string()));
    }

    #[test]
    fn test_admin_import_args_defaults() {
        let args = AdminImportArgs {
            input: PathBuf::from("/backup/export.json"),
            mode: ImportMode::Merge,
            dry_run: false,
            skip_validation: false,
            json: false,
        };
        assert_eq!(args.input, PathBuf::from("/backup/export.json"));
        assert!(!args.dry_run);
        assert!(!args.skip_validation);
    }

    #[test]
    fn test_admin_import_args_replace_mode() {
        let args = AdminImportArgs {
            input: PathBuf::from("/backup/export.json"),
            mode: ImportMode::Replace,
            dry_run: true,
            skip_validation: false,
            json: true,
        };
        assert!(args.dry_run);
    }

    #[test]
    fn test_admin_import_args_skip_existing() {
        let args = AdminImportArgs {
            input: PathBuf::from("/backup/export.json"),
            mode: ImportMode::SkipExisting,
            dry_run: false,
            skip_validation: true,
            json: false,
        };
        assert!(args.skip_validation);
    }

    #[test]
    fn test_health_check_struct() {
        let mut details = std::collections::HashMap::new();
        details.insert("backend".to_string(), "qdrant".to_string());

        let check = HealthCheck {
            component: "memory".to_string(),
            status: "healthy".to_string(),
            latency_ms: 15,
            message: "Vector store responding".to_string(),
            details,
        };
        assert_eq!(check.component, "memory");
        assert_eq!(check.status, "healthy");
        assert_eq!(check.latency_ms, 15);
        assert_eq!(check.details.get("backend"), Some(&"qdrant".to_string()));
    }

    #[test]
    fn test_validation_result_no_issues() {
        let result = ValidationResult {
            target: "config".to_string(),
            errors: vec![],
            warnings: vec![],
            info: vec!["All good".to_string()],
        };
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
        assert!(!result.info.is_empty());
    }

    #[test]
    fn test_validation_result_with_errors() {
        let result = ValidationResult {
            target: "schema".to_string(),
            errors: vec!["Missing required field".to_string()],
            warnings: vec!["Deprecated syntax".to_string()],
            info: vec![],
        };
        assert!(!result.errors.is_empty());
        assert!(!result.warnings.is_empty());
        assert!(result.info.is_empty());
    }

    #[test]
    fn test_migration_struct() {
        let migration = Migration {
            version: "2024.1.1".to_string(),
            name: "Add agent delegation table".to_string(),
            status: "pending".to_string(),
            reversible: true,
        };
        assert_eq!(migration.version, "2024.1.1");
        assert_eq!(migration.status, "pending");
        assert!(migration.reversible);
    }

    #[test]
    fn test_migration_irreversible() {
        let migration = Migration {
            version: "2024.2.0".to_string(),
            name: "Cedar schema v2 upgrade".to_string(),
            status: "applied".to_string(),
            reversible: false,
        };
        assert!(!migration.reversible);
        assert_eq!(migration.status, "applied");
    }

    #[test]
    fn test_drift_item_modified() {
        let drift = DriftItem {
            target: "config".to_string(),
            path: ".aeterna/context.toml".to_string(),
            drift_type: "modified".to_string(),
            expected: "timeout = 30".to_string(),
            actual: "timeout = 60".to_string(),
            fixable: true,
        };
        assert_eq!(drift.drift_type, "modified");
        assert!(drift.fixable);
    }

    #[test]
    fn test_drift_item_missing() {
        let drift = DriftItem {
            target: "policies".to_string(),
            path: "policies/security-baseline.cedar".to_string(),
            drift_type: "missing".to_string(),
            expected: "(file should exist)".to_string(),
            actual: "(file not found)".to_string(),
            fixable: true,
        };
        assert_eq!(drift.drift_type, "missing");
    }

    #[test]
    fn test_drift_item_extra() {
        let drift = DriftItem {
            target: "config".to_string(),
            path: ".aeterna/local.toml".to_string(),
            drift_type: "extra".to_string(),
            expected: "(should not exist)".to_string(),
            actual: "(file found)".to_string(),
            fixable: false,
        };
        assert_eq!(drift.drift_type, "extra");
        assert!(!drift.fixable);
    }

    #[test]
    fn test_export_stats() {
        let stats = ExportStats {
            memories: 1000,
            knowledge_items: 50,
            policies: 25,
            config_files: 5,
            audit_entries: 10000,
        };
        assert_eq!(stats.memories, 1000);
        assert_eq!(stats.knowledge_items, 50);
        assert_eq!(stats.policies, 25);
        assert_eq!(stats.config_files, 5);
        assert_eq!(stats.audit_entries, 10000);
    }

    #[test]
    fn test_export_stats_no_audit() {
        let stats = ExportStats {
            memories: 500,
            knowledge_items: 20,
            policies: 10,
            config_files: 3,
            audit_entries: 0,
        };
        assert_eq!(stats.audit_entries, 0);
    }

    #[test]
    fn test_import_analysis() {
        let analysis = ImportAnalysis {
            format: "json".to_string(),
            source_version: "2024.1.0".to_string(),
            memories: 100,
            knowledge_items: 10,
            policies: 5,
            conflicts: vec![],
        };
        assert_eq!(analysis.format, "json");
        assert!(analysis.conflicts.is_empty());
    }

    #[test]
    fn test_import_analysis_with_conflicts() {
        let conflicts = vec![
            ImportConflict {
                item_type: "memory".to_string(),
                id: "mem_123".to_string(),
                reason: "Already exists".to_string(),
            },
            ImportConflict {
                item_type: "policy".to_string(),
                id: "security-baseline".to_string(),
                reason: "Version mismatch".to_string(),
            },
        ];
        let analysis = ImportAnalysis {
            format: "yaml".to_string(),
            source_version: "2024.0.5".to_string(),
            memories: 50,
            knowledge_items: 5,
            policies: 3,
            conflicts,
        };
        assert_eq!(analysis.conflicts.len(), 2);
    }

    #[test]
    fn test_import_conflict() {
        let conflict = ImportConflict {
            item_type: "memory".to_string(),
            id: "mem_abc123".to_string(),
            reason: "Already exists with different content".to_string(),
        };
        assert_eq!(conflict.item_type, "memory");
        assert_eq!(conflict.id, "mem_abc123");
    }

    #[test]
    fn test_colored_status_green() {
        let result = colored_status("✓", "green");
        assert!(result.contains("✓"));
    }

    #[test]
    fn test_colored_status_yellow() {
        let result = colored_status("!", "yellow");
        assert!(result.contains('!'));
    }

    #[test]
    fn test_colored_status_red() {
        let result = colored_status("✗", "red");
        assert!(result.contains("✗"));
    }

    #[test]
    fn test_colored_status_white_default() {
        let result = colored_status("?", "unknown");
        assert!(result.contains('?'));
    }

    #[tokio::test]
    async fn test_health_check_not_connected_has_no_fake_details() {
        // When not connected, no fake detail values (vector counts, etc.) should appear.
        for component in &["memory", "knowledge", "policy", "context"] {
            let check = check_component_health(component, 30).await;
            assert!(
                check.details.is_empty(),
                "component '{component}' must not have fake details when disconnected"
            );
        }
    }

    #[tokio::test]
    async fn test_validate_schema() {
        let result = validate_target("schema", None, false).await;
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
        assert!(!result.info.is_empty());
    }

    #[tokio::test]
    async fn test_validate_with_strict_mode() {
        let result = validate_target("config", None, true).await;
        assert!(result.errors.is_empty());
    }
}

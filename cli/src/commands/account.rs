use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use serde_json::json;

#[derive(Subcommand)]
pub enum AccountCommand {
    #[command(about = "List accounts")]
    List(AccountListArgs),
    #[command(about = "Show account details")]
    Show(AccountShowArgs),
    #[command(about = "Create an account")]
    Create(AccountCreateArgs),
    #[command(about = "Update an account")]
    Update(AccountUpdateArgs),
    #[command(about = "Delete an account")]
    Delete(AccountDeleteArgs),
    #[command(about = "List tenants attached to an account")]
    Tenants(AccountTenantsArgs),
    #[command(about = "Attach a tenant to an account")]
    Attach(AccountAttachArgs),
    #[command(about = "Detach a tenant from its account")]
    Detach(AccountDetachArgs),
}

#[derive(Args)]
pub struct AccountListArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct AccountShowArgs {
    pub account: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct AccountCreateArgs {
    pub slug: String,
    pub name: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct AccountUpdateArgs {
    pub account: String,
    #[arg(long)]
    pub slug: Option<String>,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct AccountDeleteArgs {
    pub account: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct AccountTenantsArgs {
    pub account: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct AccountAttachArgs {
    pub tenant: String,
    pub account_id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct AccountDetachArgs {
    pub tenant: String,
    #[arg(long)]
    pub json: bool,
}

pub async fn run(cmd: AccountCommand) -> Result<()> {
    match cmd {
        AccountCommand::List(args) => run_list(args).await,
        AccountCommand::Show(args) => run_show(args).await,
        AccountCommand::Create(args) => run_create(args).await,
        AccountCommand::Update(args) => run_update(args).await,
        AccountCommand::Delete(args) => run_delete(args).await,
        AccountCommand::Tenants(args) => run_tenants(args).await,
        AccountCommand::Attach(args) => run_attach(args).await,
        AccountCommand::Detach(args) => run_detach(args).await,
    }
}

async fn client() -> Result<crate::client::AeternaClient> {
    let resolved = crate::profile::load_resolved(None, None)
        .context("No active profile/server configuration found")?;
    crate::client::AeternaClient::from_profile(&resolved)
        .await
        .context("Failed to create authenticated client")
}

fn print_json(value: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

async fn run_list(args: AccountListArgs) -> Result<()> {
    let client = client().await?;
    let value: serde_json::Value = client.get("/api/v1/admin/accounts").await?.json().await?;
    if args.json {
        return print_json(&value);
    }
    print_json(&value)
}

async fn run_show(args: AccountShowArgs) -> Result<()> {
    let client = client().await?;
    let value: serde_json::Value = client
        .get(&format!("/api/v1/admin/accounts/{}", args.account))
        .await?
        .json()
        .await?;
    if args.json {
        return print_json(&value);
    }
    print_json(&value)
}

async fn run_create(args: AccountCreateArgs) -> Result<()> {
    let client = client().await?;
    let body = json!({ "slug": args.slug, "name": args.name });
    let value: serde_json::Value = client
        .post("/api/v1/admin/accounts", &body)
        .await?
        .json()
        .await?;
    if args.json {
        return print_json(&value);
    }
    print_json(&value)
}

async fn run_update(args: AccountUpdateArgs) -> Result<()> {
    let client = client().await?;
    let body = json!({ "slug": args.slug, "name": args.name });
    let value: serde_json::Value = client
        .patch(&format!("/api/v1/admin/accounts/{}", args.account), &body)
        .await?
        .json()
        .await?;
    if args.json {
        return print_json(&value);
    }
    print_json(&value)
}

async fn run_delete(args: AccountDeleteArgs) -> Result<()> {
    let client = client().await?;
    let value: serde_json::Value = client
        .delete(&format!("/api/v1/admin/accounts/{}", args.account))
        .await?
        .json()
        .await?;
    if args.json {
        return print_json(&value);
    }
    print_json(&value)
}

async fn run_tenants(args: AccountTenantsArgs) -> Result<()> {
    let client = client().await?;
    let value: serde_json::Value = client
        .get(&format!("/api/v1/admin/accounts/{}/tenants", args.account))
        .await?
        .json()
        .await?;
    if args.json {
        return print_json(&value);
    }
    print_json(&value)
}

async fn run_attach(args: AccountAttachArgs) -> Result<()> {
    let client = client().await?;
    let body = json!({ "accountId": args.account_id });
    let value: serde_json::Value = client
        .post(
            &format!("/api/v1/admin/tenants/{}/account", args.tenant),
            &body,
        )
        .await?
        .json()
        .await?;
    if args.json {
        return print_json(&value);
    }
    print_json(&value)
}

async fn run_detach(args: AccountDetachArgs) -> Result<()> {
    let client = client().await?;
    let value: serde_json::Value = client
        .delete(&format!("/api/v1/admin/tenants/{}/account", args.tenant))
        .await?
        .json()
        .await?;
    if args.json {
        return print_json(&value);
    }
    print_json(&value)
}

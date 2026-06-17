use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

mod backend;
mod client;
mod commands;
mod credentials;
pub mod env_vars;
pub mod exit_code;
mod offline;
mod output;
mod profile;
mod secret_input;
mod server;
pub mod ux_error;

use commands::{Cli, Commands};

/// Reject the deprecated `--token` / `--token=...` CLI flag (B2 §10.6).
///
/// The pre-§10 CLI accepted a `--token <value>` flag on several
/// subcommands, which meant operators habitually pasted raw bearer
/// tokens onto the command line — where they landed in shell history,
/// `ps aux`, and CI logs. The supported surface is now:
///
/// 1. `AETERNA_API_TOKEN` env var — for CI and scripted callers.
/// 2. `aeterna auth login` → `~/.config/aeterna/credentials.toml`
///    — for interactive operators.
/// 3. OS keychain (future work, tracked separately in `credentials.rs`).
///
/// Rather than letting clap silently ignore `--token` (it is not
/// declared anywhere, so clap would emit a generic "unexpected
/// argument" message and exit 2), we scan argv ourselves and emit a
/// helpful migration message naming both supported paths.
///
/// We match on:
///
/// - `--token` as a standalone argument (value in next position),
/// - `--token=...` (value inline).
///
/// The short form `-t` is **intentionally not matched** here: several
/// legitimate subcommands use `-t` as a short alias for an unrelated
/// flag (`govern pending -t <type>`, `memory ... -t <tag>`,
/// `knowledge ... -t <tag>`), and rejecting `-t` at the pre-clap
/// layer produced friendly-fire failures against those commands.
/// The `--token` long form was the only universally-documented
/// legacy surface, so blocking only `--token` / `--token=` keeps the
/// security guarantee without colliding with subcommand shorts.
///
/// Returns `Some(&str)` with the offending form for the test suite
/// to assert against; returns `None` when argv is clean.
///
/// Scans but does **not** inspect values. The value itself might be
/// a real token — we deliberately avoid reading it, printing it, or
/// including it in the error message, so a user who pastes a command
/// with `--token sk-live-...` into a support channel does not leak
/// the token in their own pasted error output.
fn reject_legacy_token_flag<I, S>(args: I) -> Option<&'static str>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    for arg in args {
        let a = arg.as_ref();
        if a == "--token" {
            return Some("--token");
        }
        if a.starts_with("--token=") {
            return Some("--token=");
        }
        // `-t` / `-t=` deliberately NOT matched — see doc-comment above.
    }
    None
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    // B2 §10.6 — scan argv for the deprecated `--token` surface
    // BEFORE clap parses. Clap would otherwise emit a generic
    // "unexpected argument" error that buries the migration guidance.
    // We skip argv[0] (the binary name) because an operator running
    // the binary from a path containing "--token" (unlikely but
    // possible) would get a spurious rejection otherwise.
    if let Some(form) = reject_legacy_token_flag(std::env::args().skip(1)) {
        ux_error::UxError::new(format!("The `{form}` CLI flag is no longer accepted."))
            .why(
                "Passing bearer tokens on the command line leaks them into shell \
             history, process listings, and CI logs. The CLI now reads tokens \
             only from the environment or from the credentials file written by \
             `aeterna auth login`.",
            )
            .fix("For scripted / CI use: export AETERNA_API_TOKEN=<service-token>")
            .fix("For interactive use: run `aeterna auth login`")
            .fix(
                "Service tokens are minted via `aeterna auth token create` \
             (PlatformAdmin only; see B2 §10.2).",
            )
            .display();
        exit_code::ExitCode::Usage.exit();
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::Init(args) => commands::init::run(args),
        Commands::Status(args) => commands::status::run(args),
        Commands::Sync(args) => commands::sync::run(args).await,
        Commands::Check(args) => commands::check::run(args).await,
        Commands::Context(args) => commands::context::run(args),
        Commands::Hints(args) => commands::hints::run(args),
        Commands::Memory(cmd) => commands::memory::run(cmd).await,
        Commands::Knowledge(cmd) => commands::knowledge::run(cmd).await,
        Commands::Policy(cmd) => commands::policy::run(cmd).await,
        Commands::Account(cmd) => commands::account::run(cmd).await,
        Commands::Org(cmd) => commands::org::run(cmd).await,
        Commands::Team(cmd) => commands::team::run(cmd).await,
        Commands::Tenant(cmd) => commands::tenant::run(cmd).await,
        Commands::User(cmd) => commands::user::run(cmd).await,
        Commands::Agent(cmd) => commands::agent::run(cmd).await,
        Commands::Govern(cmd) => commands::govern::run(cmd).await,
        Commands::Admin(cmd) => commands::admin::run(cmd).await,
        Commands::CodeSearch(cmd) => commands::search::handle_command(cmd).await,
        Commands::Completion(args) => commands::completion::run(args),
        Commands::Setup(args) => commands::setup::run(args).await,
        Commands::Serve(args) => commands::serve::run(args).await,
        Commands::Auth(cmd) => commands::auth::run(cmd).await,
        Commands::Config(cmd) => commands::config::run(cmd).await,
        Commands::Profile(cmd) => commands::profile::run(cmd).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // B2 §10.6 — reject legacy --token flag
    // ---------------------------------------------------------------

    #[test]
    fn clean_argv_is_accepted() {
        // No token flag anywhere — the scanner must return None so
        // main proceeds to clap. We cover both long and short command
        // shapes and a value that happens to contain the substring
        // "token" to confirm we are not matching too greedily.
        let cases: &[&[&str]] = &[
            &["tenant", "apply", "-f", "manifest.json"],
            &["auth", "login", "--profile", "prod"],
            &["memory", "search", "--query", "my-api-token-policy"],
            &["admin", "export", "--output", "backup.zst"],
            &[], // empty argv — edge case
        ];
        for argv in cases {
            assert_eq!(
                reject_legacy_token_flag(argv.iter().copied()),
                None,
                "clean argv {argv:?} must not trigger rejection"
            );
        }
    }

    #[test]
    fn standalone_long_flag_is_rejected() {
        let argv = ["tenant", "list", "--token", "abc123"];
        assert_eq!(
            reject_legacy_token_flag(argv.iter().copied()),
            Some("--token")
        );
    }

    #[test]
    fn inline_long_flag_is_rejected() {
        // `--token=<value>` is the form clap would have accepted
        // historically; catching it by the `=` prefix ensures we do
        // not read the value itself.
        let argv = ["tenant", "list", "--token=abc123"];
        assert_eq!(
            reject_legacy_token_flag(argv.iter().copied()),
            Some("--token=")
        );
    }

    #[test]
    fn standalone_short_flag_is_accepted() {
        // `-t` is a legitimate short alias in several subcommands
        // (`govern pending -t <type>`, `memory -t <tag>`,
        // `knowledge -t <tag>`). The pre-clap legacy-token scanner
        // must NOT steal `-t` from those commands; rejection is
        // scoped to the unambiguous `--token` long form.
        let argv = ["govern", "pending", "-t", "policy"];
        assert_eq!(reject_legacy_token_flag(argv.iter().copied()), None);
    }

    #[test]
    fn inline_short_flag_is_accepted() {
        // Same rationale as `standalone_short_flag_is_accepted`:
        // `-t=...` is not reserved by the legacy-token guard.
        let argv = ["govern", "pending", "-t=policy"];
        assert_eq!(reject_legacy_token_flag(argv.iter().copied()), None);
    }

    #[test]
    fn rejection_does_not_leak_token_value() {
        // Regression guard: the scanner returns only a fixed-shape
        // identifier, never the caller's actual token. A user pasting
        // the rejection output into a support channel must not also
        // leak their credential.
        let secret = "sk-live-must-not-appear";
        let argv = ["--token", secret];
        let form = reject_legacy_token_flag(argv.iter().copied()).unwrap();
        assert_eq!(form, "--token");
        // The returned &'static str is a compile-time constant; it
        // literally cannot contain the secret. The test asserts the
        // *shape* of the guarantee, not the value.
        assert!(!form.contains(secret));
        assert!(!form.contains("sk-"));
    }

    #[test]
    fn first_match_wins_when_flag_appears_twice() {
        // Two different shapes of the flag on the same command line —
        // we report the first one so the error message stays
        // deterministic. Either result would technically be
        // correct but deterministic output is easier to test and
        // easier to reason about when a user reads the error.
        let argv = ["--token=a", "cmd", "--token", "b"];
        assert_eq!(
            reject_legacy_token_flag(argv.iter().copied()),
            Some("--token=")
        );
    }

    #[test]
    fn substring_match_is_not_triggered() {
        // `--token-file`, `--service-token`, `--use-token=yes` etc.
        // are NOT legacy `--token` — they are either nonexistent
        // flags (clap will reject) or legitimate neighbouring flags.
        // The scanner must match the exact forms only.
        let cases: &[&[&str]] = &[
            &["--token-file", "/tmp/t"],
            &["--service-token"],
            &["--use-token=yes"],
            &["--tokenize"],
            &["token"],  // bare positional
            &["-tx"],    // short-flag cluster that is not `-t`
            &["-token"], // single-dash long name — not our shape
        ];
        for argv in cases {
            assert_eq!(
                reject_legacy_token_flag(argv.iter().copied()),
                None,
                "argv {argv:?} must not be matched as legacy --token"
            );
        }
    }
}

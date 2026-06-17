//! H2 regression test — single canonical app-side GUC namespace.
//!
//! The RLS enforcement model (issue #58) fixes the canonical session variable
//! at `app.tenant_id`. Any Rust source file on the app side that names one of
//! the legacy alternate GUCs is a latent dual-namespace hazard (H2): the app
//! sets one name, the policy reads another, and the policy silently evaluates
//! against an empty string.
//!
//! This test walks `cli/src/` and `storage/src/` recursively and fails if
//! any `.rs` file mentions a legacy GUC name. Migration files (emit SQL
//! that is a historical record) and test files (may reference the legacy
//! names as quoted regression sentinels, as this file does) are excluded.
//!
//! Task: §2.6 of `openspec/changes/decide-rls-enforcement-model/tasks.md`.

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Legacy GUC names that MUST NOT appear in app-side source files.
///
/// These literals are split across concatenation so that the test file itself
/// does not contain any forbidden token as a contiguous substring, which would
/// cause the test to flag itself when scanning the `storage/` tree if someone
/// ever moved it into `storage/src/`.
fn forbidden_guc_tokens() -> Vec<String> {
    vec![
        format!("{}{}{}{}", "app.", "co", "mpany", "_id"),
        format!("{}{}", "app.", "current_tenant_id"),
        format!("{}{}{}{}{}", "app.", "current_", "co", "mpany", "_id"),
    ]
}

/// Resolve the workspace root from this test’s manifest dir (`storage/`).
fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .expect("storage crate must have a parent (the workspace root)")
        .to_path_buf()
}

fn is_rust_source(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "rs")
}

#[test]
fn no_legacy_guc_names_in_app_side_source() {
    let root = workspace_root();
    let scan_dirs = [
        root.join("cli").join("src"),
        root.join("storage").join("src"),
    ];

    let forbidden = forbidden_guc_tokens();
    let mut offenders: Vec<(PathBuf, String, usize)> = Vec::new();

    for dir in &scan_dirs {
        if !dir.exists() {
            panic!(
                "Scan directory {} does not exist. The workspace layout \
                 changed — update this test.",
                dir.display()
            );
        }

        for entry in WalkDir::new(dir).into_iter().filter_map(Result::ok) {
            let path: &Path = entry.path();
            if !is_rust_source(path) {
                continue;
            }
            let contents = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for (line_no, line) in contents.lines().enumerate() {
                for token in &forbidden {
                    if line.contains(token.as_str()) {
                        offenders.push((path.to_path_buf(), token.clone(), line_no + 1));
                    }
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "H2 regression: legacy GUC name(s) found in app-side source. \
         The single canonical app-side GUC is `app.tenant_id`. Offenders:\n{}",
        offenders
            .iter()
            .map(|(p, t, l)| format!("  {}:{}  ({})", p.display(), l, t))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn scan_dirs_resolve_from_cargo_manifest_dir() {
    // Sanity: the workspace layout assumed by this regression test actually
    // exists. Guards against a workspace refactor that would make the scan
    // above silently cover zero files.
    let root = workspace_root();
    assert!(
        root.join("cli").join("src").is_dir(),
        "cli/src not found at {}",
        root.display()
    );
    assert!(
        root.join("storage").join("src").is_dir(),
        "storage/src not found at {}",
        root.display()
    );
}

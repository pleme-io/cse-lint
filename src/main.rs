//! cse-lint — Constructive Substrate Engineering audit linter.
//!
//! Walks every repo under a workspace root and asserts the four CSE
//! invariants:
//!   1. CLAUDE.md links to the CSE methodology + Compounding Directive
//!   2. flake.nix consumes a substrate helper (no hand-rolled
//!      buildRustPackage)
//!   3. Apps in the typed manifest have class assignments referenced
//!      by at least one profile
//!   4. flake.nix passes `module = { ... }` (no legacy
//!      `// { homeManagerModules.default = import ./module ... }`)
//!
//! Designed per the Compounding Directive's measurement requirement.
//! Adherence is mechanically checkable rather than aspirational.

mod check;
mod model;
mod report;
mod source;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::check::{
    ClaudeMdPointerChecker, CseChecker, HandRollDetectionChecker,
    ManifestMembershipChecker, ModuleTrioAdoptionChecker,
};
use crate::model::{CseAuditReport, CseCheckKind, RepoResult};
use crate::source::{FilesystemSource, RepoSource};

#[derive(Parser)]
#[command(
    name = "cse-lint",
    version,
    about = "Audit pleme-io repos for Constructive Substrate Engineering adherence."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Audit repos under a workspace root.
    Audit {
        /// Workspace root (e.g. ~/code/github/pleme-io).
        #[arg(default_value = "~/code/github/pleme-io")]
        root: String,

        /// Optional repo filter — audit only these names. Comma-separated.
        #[arg(long)]
        only: Option<String>,

        /// Output JSON instead of human-readable.
        #[arg(long)]
        json: bool,

        /// Exit non-zero if any violations are found.
        #[arg(long)]
        strict: bool,
    },
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(stripped) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    PathBuf::from(p)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Audit { root, only, json, strict } => {
            let root = expand_tilde(&root);
            let only_list: Vec<String> = only
                .map(|s| s.split(',').map(|p| p.trim().to_string()).collect())
                .unwrap_or_default();
            let source = FilesystemSource::new(root.clone()).with_only(only_list);
            let manifest_path = root.join("nix/lib/ecosystem.nix");
            let manifest_path = if manifest_path.exists() { Some(manifest_path) } else { None };

            let checkers: Vec<Box<dyn CseChecker>> = vec![
                Box::new(ClaudeMdPointerChecker),
                Box::new(HandRollDetectionChecker),
                Box::new(ManifestMembershipChecker { manifest_path }),
                Box::new(ModuleTrioAdoptionChecker),
            ];

            let repos = source.repos()?;
            let mut results: Vec<RepoResult> = Vec::with_capacity(repos.len());
            let mut totals: BTreeMap<CseCheckKind, usize> = BTreeMap::new();
            for kind in CseCheckKind::all() {
                totals.insert(kind, 0);
            }

            for repo in &repos {
                let mut violations = Vec::new();
                let mut checks_run = Vec::new();
                for checker in &checkers {
                    checker.check(repo, &mut violations);
                    checks_run.push(checker.kind());
                }
                for v in &violations {
                    *totals.entry(v.kind()).or_insert(0) += 1;
                }
                let pass = violations.is_empty();
                results.push(RepoResult {
                    repo_name: repo.name.clone(),
                    violations,
                    checks_run,
                    pass,
                });
            }

            let passing = results.iter().filter(|r| r.pass).count();
            let report = CseAuditReport {
                total_repos: results.len(),
                passing_repos: passing,
                repos: results,
                violations_by_kind: totals,
                run_at: chrono_now(),
            };

            if json {
                println!("{}", report::render_json(&report)?);
            } else {
                print!("{}", report::render_human(&report));
            }

            let total_violations: usize = report.violations_by_kind.values().sum();
            if strict && total_violations > 0 {
                std::process::exit(1);
            }
            Ok(())
        }
    }
}

/// Lightweight ISO-8601 timestamp without dragging in chrono.
fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Format as `<unix>` for now — operators see relative freshness;
    // a proper ISO-8601 needs chrono. Worth the tradeoff for a tiny
    // dep-free binary.
    format!("unix:{secs}")
}

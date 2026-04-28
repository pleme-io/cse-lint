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
mod fix;
mod model;
mod render;
mod report;
mod scaffold;
mod source;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::check::{
    CaixaNaiveteChecker, ClaudeMdPointerChecker, CseChecker, DeploymentCoverageChecker,
    HandRollDetectionChecker, ManifestMembershipChecker,
    ModuleTrioAdoptionChecker,
};
use crate::fix::{all_remediators, apply_edit, EditAction, PlannedEdit};
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

#[derive(Copy, Clone, ValueEnum)]
enum RenderFormat {
    /// Markdown table for embedding in docs / CLAUDE.md.
    Markdown,
    /// JSON pretty-printed (pipe through `jq`).
    Json,
    /// Graphviz DOT graph (render with `dot -Tpng`).
    Dot,
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

    /// Render the typed ecosystem manifest as a queryable artifact.
    Render {
        /// Workspace root.
        #[arg(default_value = "~/code/github/pleme-io")]
        root: String,

        /// Output format.
        #[arg(value_enum)]
        format: RenderFormat,
    },

    /// Scaffold baseline CLAUDE.md (with CSE pointer pre-wired) for
    /// repos that lack one. Defaults to dry-run.
    Scaffold {
        /// Workspace root.
        #[arg(default_value = "~/code/github/pleme-io")]
        root: String,

        /// Optional repo filter — comma-separated names.
        #[arg(long)]
        only: Option<String>,

        /// Actually create files. Without this, prints planned scaffolds.
        #[arg(long)]
        apply: bool,

        /// Auto-commit each scaffolded repo (requires --apply).
        #[arg(long)]
        commit: bool,
    },

    /// Auto-remediate violations. Defaults to dry-run.
    Fix {
        /// Workspace root.
        #[arg(default_value = "~/code/github/pleme-io")]
        root: String,

        /// Optional repo filter.
        #[arg(long)]
        only: Option<String>,

        /// Restrict fixes to one or more check kinds (comma-separated).
        /// Available: claude-md-pointer, hand-roll, manifest-membership,
        /// module-trio-adoption. Default: all auto-fixable kinds.
        #[arg(long)]
        check: Option<String>,

        /// Actually write changes. Without this, prints planned edits
        /// without modifying files.
        #[arg(long)]
        apply: bool,

        /// Auto-commit each repo after applying fix (requires --apply).
        /// Commit message: "CLAUDE.md: cse-lint added CSE pointer".
        /// Push is NOT performed — operator reviews then pushes
        /// manually.
        #[arg(long)]
        commit: bool,
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
                Box::new(ManifestMembershipChecker::new(manifest_path.clone())),
                Box::new(ModuleTrioAdoptionChecker),
                Box::new(DeploymentCoverageChecker {
                    workspace_root: Some(root.clone()),
                }),
                Box::new(CaixaNaiveteChecker),
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
        Command::Render { root, format } => {
            let root = expand_tilde(&root);
            let manifest_path = root.join("nix/lib/ecosystem.nix");
            let manifest = render::load_manifest(&manifest_path)?;
            let out = match format {
                RenderFormat::Markdown => render::render_markdown_table(&manifest),
                RenderFormat::Json => render::render_json(&manifest)?,
                RenderFormat::Dot => render::render_dot(&manifest),
            };
            print!("{}", out);
            Ok(())
        }
        Command::Scaffold { root, only, apply, commit } => {
            let root = expand_tilde(&root);
            let only_list: Vec<String> = only
                .map(|s| s.split(',').map(|p| p.trim().to_string()).collect())
                .unwrap_or_default();
            let source = FilesystemSource::new(root.clone()).with_only(only_list);
            let repos = source.repos()?;

            let plans: Vec<scaffold::ScaffoldPlan> = repos
                .iter()
                .filter_map(|r| scaffold::plan_scaffold(r))
                .collect();

            println!(
                "cse-lint scaffold — planned {} CLAUDE.md file(s).",
                plans.len()
            );
            if !apply {
                println!("(dry-run; pass --apply to create files)\n");
            } else {
                println!("(--apply set; creating files)\n");
            }

            let mut applied: usize = 0;
            for plan in &plans {
                println!("  ▸ {} ← scaffold {} bytes", plan.repo_name, plan.content.len());
                if apply {
                    match scaffold::apply_scaffold(plan) {
                        Ok(()) => {
                            applied += 1;
                            if commit {
                                let _ = std::process::Command::new("git")
                                    .current_dir(&root.join(&plan.repo_name))
                                    .args(&["add", "CLAUDE.md"])
                                    .status();
                                let _ = std::process::Command::new("git")
                                    .current_dir(&root.join(&plan.repo_name))
                                    .args(&[
                                        "-c", "commit.gpgsign=false",
                                        "commit", "-m",
                                        "CLAUDE.md: cse-lint scaffolded baseline with CSE pointer",
                                    ])
                                    .status();
                            }
                        }
                        Err(e) => eprintln!("    error: {}", e),
                    }
                }
            }

            if apply {
                println!(
                    "\nScaffolded {} file(s){}.",
                    applied,
                    if commit { ", each committed" } else { "; commit manually after review" },
                );
            }
            Ok(())
        }
        Command::Fix { root, only, check, apply, commit } => {
            let root = expand_tilde(&root);
            let only_list: Vec<String> = only
                .map(|s| s.split(',').map(|p| p.trim().to_string()).collect())
                .unwrap_or_default();
            let kind_filter: Option<Vec<CseCheckKind>> = check.as_ref().map(|s| {
                s.split(',')
                    .filter_map(|c| match c.trim() {
                        "claude-md-pointer" => Some(CseCheckKind::ClaudeMdPointer),
                        "hand-roll" => Some(CseCheckKind::HandRollDetection),
                        "manifest-membership" => Some(CseCheckKind::ManifestMembership),
                        "module-trio-adoption" => Some(CseCheckKind::ModuleTrioAdoption),
                        "deployment-coverage" => Some(CseCheckKind::DeploymentCoverage),
                        _ => None,
                    })
                    .collect()
            });

            let source = FilesystemSource::new(root.clone()).with_only(only_list);
            let manifest_path = root.join("nix/lib/ecosystem.nix");
            let manifest_path = if manifest_path.exists() { Some(manifest_path) } else { None };

            let checkers: Vec<Box<dyn CseChecker>> = vec![
                Box::new(ClaudeMdPointerChecker),
                Box::new(HandRollDetectionChecker),
                Box::new(ManifestMembershipChecker::new(manifest_path.clone())),
                Box::new(ModuleTrioAdoptionChecker),
                Box::new(DeploymentCoverageChecker {
                    workspace_root: Some(root.clone()),
                }),
                Box::new(CaixaNaiveteChecker),
            ];
            let remediators = all_remediators();

            let repos = source.repos()?;
            let mut planned_edits: Vec<PlannedEdit> = Vec::new();
            let mut skipped_count: usize = 0;

            for repo in &repos {
                let mut violations = Vec::new();
                for checker in &checkers {
                    if let Some(filter) = &kind_filter {
                        if !filter.contains(&checker.kind()) {
                            continue;
                        }
                    }
                    checker.check(repo, &mut violations);
                }
                for v in &violations {
                    for r in &remediators {
                        if r.kind() != v.kind() {
                            continue;
                        }
                        if let Some(edit) = r.plan(repo, v) {
                            match &edit.action {
                                EditAction::Skip { .. } => skipped_count += 1,
                                _ => {}
                            }
                            planned_edits.push(edit);
                        }
                    }
                }
            }

            // Print plan
            let applicable_count = planned_edits
                .iter()
                .filter(|e| !matches!(e.action, EditAction::Skip { .. }))
                .count();

            println!(
                "cse-lint fix — planned {} edit(s), {} skipped (need manual handling).",
                applicable_count, skipped_count,
            );
            if !apply {
                println!("(dry-run; pass --apply to write changes)\n");
            } else {
                println!("(--apply set; writing changes)\n");
            }

            let mut applied_count: usize = 0;
            for edit in &planned_edits {
                match &edit.action {
                    EditAction::InsertAfterFirstMatch { .. } => {
                        println!(
                            "  [{}] {} ← insert pointer block",
                            edit.kind.label(),
                            edit.path.display(),
                        );
                        if apply {
                            match apply_edit(edit) {
                                Ok(true) => applied_count += 1,
                                Ok(false) => {}
                                Err(e) => eprintln!("    error: {}", e),
                            }
                            if commit {
                                let _ = std::process::Command::new("git")
                                    .current_dir(&root.join(&edit.repo_name))
                                    .args(&["add", "CLAUDE.md"])
                                    .status();
                                let _ = std::process::Command::new("git")
                                    .current_dir(&root.join(&edit.repo_name))
                                    .args(&[
                                        "-c", "commit.gpgsign=false",
                                        "commit", "-m",
                                        "CLAUDE.md: cse-lint added CSE pointer",
                                    ])
                                    .status();
                            }
                        }
                    }
                    EditAction::Skip { reason } => {
                        println!(
                            "  [{}] {} (skip: {})",
                            edit.kind.label(),
                            edit.repo_name,
                            reason,
                        );
                    }
                }
            }

            if apply {
                println!(
                    "\nApplied {} edit(s){}.",
                    applied_count,
                    if commit { ", each committed" } else { "; commit manually after review" },
                );
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

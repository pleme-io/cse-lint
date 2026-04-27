//! Auto-remediation primitives for cse-lint.
//!
//! Per the Compounding Directive: cse-lint doesn't just *find*
//! violations, it produces typed remediations the operator (or an
//! agent, or CI) can apply mechanically. Each [`Remediator`] is a
//! small impl that knows how to fix one kind of violation.
//!
//! Fixes default to dry-run. Apply requires `--apply`.

use crate::model::{CseCheckKind, CseViolation, RepoContext};
use anyhow::Result;
use std::path::PathBuf;

/// A planned file edit. The fix tool collects edits, prints them in
/// dry-run, applies them with `--apply`.
#[derive(Debug, Clone)]
pub struct PlannedEdit {
    pub repo_name: String,
    pub path: PathBuf,
    pub kind: CseCheckKind,
    pub action: EditAction,
}

#[derive(Debug, Clone)]
pub enum EditAction {
    /// Insert `text` immediately after the first line that matches `after_first_line_match`.
    /// If no match, insert at top.
    InsertAfterFirstMatch {
        after_first_line_match: String,
        text: String,
    },
    /// Skip — fix is intentionally not auto-applicable for this violation.
    Skip { reason: String },
}

pub trait Remediator: Send + Sync {
    fn kind(&self) -> CseCheckKind;
    fn plan(&self, repo: &RepoContext, violation: &CseViolation) -> Option<PlannedEdit>;
}

// ─── 1. CSE pointer remediator ────────────────────────────────────────
pub struct ClaudeMdPointerRemediator;

/// Standard, generic CSE pointer block. Operators can post-edit the
/// hook sentence per-repo — but having ANY pointer is better than
/// having none.
pub fn standard_pointer_block() -> &'static str {
    "> **★★★ CSE / Knowable Construction.** This repo operates under \
**Constructive Substrate Engineering** — canonical specification at \
[`pleme-io/theory/CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md`](https://github.com/pleme-io/theory/blob/main/CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md). \
The Compounding Directive (operational rules: solve once, load-bearing \
fixes only, idiom-first, models stay current, direction beats velocity) \
is in the org-level pleme-io/CLAUDE.md ★★★ section. Read both before \
non-trivial changes.\n"
}

impl Remediator for ClaudeMdPointerRemediator {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::ClaudeMdPointer
    }

    fn plan(&self, repo: &RepoContext, violation: &CseViolation) -> Option<PlannedEdit> {
        // Only handle MissingCsePointer violations.
        let CseViolation::MissingCsePointer { .. } = violation else {
            return None;
        };
        let claude = repo.claude_md.as_deref()?;
        // Skip if no H1 line — the file is too unusual to auto-edit.
        let first_h1 = claude.lines().find(|l| l.starts_with("# "))?;

        // Build the pointer block as a quoted Markdown blockquote with a
        // trailing blank line so it doesn't run into the next paragraph.
        let pointer = format!("\n{}\n", standard_pointer_block());

        Some(PlannedEdit {
            repo_name: repo.name.clone(),
            path: repo.path.join("CLAUDE.md"),
            kind: CseCheckKind::ClaudeMdPointer,
            action: EditAction::InsertAfterFirstMatch {
                after_first_line_match: first_h1.to_string(),
                text: pointer,
            },
        })
    }
}

// ─── 2. Hand-roll remediator (skip — needs human judgment) ────────────
pub struct HandRollRemediator;

impl Remediator for HandRollRemediator {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::HandRollDetection
    }

    fn plan(&self, repo: &RepoContext, violation: &CseViolation) -> Option<PlannedEdit> {
        let CseViolation::HandRoll { .. } = violation else {
            return None;
        };
        Some(PlannedEdit {
            repo_name: repo.name.clone(),
            path: repo.path.join("flake.nix"),
            kind: CseCheckKind::HandRollDetection,
            action: EditAction::Skip {
                reason: "Migrating to a substrate helper requires per-repo judgment about which builder fits (rust-tool / rust-workspace / rust-service / typescript-tool / etc.). Skipping — handle manually with substrate/lib/build/<lang>/<helper>.nix.".into(),
            },
        })
    }
}

// ─── 3. Legacy module pattern remediator (skip — judgment needed) ─────
pub struct LegacyModuleRemediator;

impl Remediator for LegacyModuleRemediator {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::ModuleTrioAdoption
    }

    fn plan(&self, repo: &RepoContext, violation: &CseViolation) -> Option<PlannedEdit> {
        let CseViolation::LegacyModulePattern { .. } = violation else {
            return None;
        };
        Some(PlannedEdit {
            repo_name: repo.name.clone(),
            path: repo.path.join("flake.nix"),
            kind: CseCheckKind::ModuleTrioAdoption,
            action: EditAction::Skip {
                reason: "Migrating from `// { homeManagerModules.default = import ./module ... }` to `module = { ... }` requires reading the module's option surface to fill in withMcp / withHttp / withUserDaemon / shikumiTypedGroups / hmNamespace correctly. Skipping — see substrate/lib/module-trio.nix spec and commit nami@a2a2a72 for canonical example.".into(),
            },
        })
    }
}

// ─── 4. Manifest membership remediator (skip — needs class decision) ──
pub struct ManifestMembershipRemediator;

impl Remediator for ManifestMembershipRemediator {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::ManifestMembership
    }

    fn plan(&self, repo: &RepoContext, violation: &CseViolation) -> Option<PlannedEdit> {
        let CseViolation::ManifestInconsistency { .. } = violation else {
            return None;
        };
        Some(PlannedEdit {
            repo_name: repo.name.clone(),
            path: PathBuf::from("nix/lib/ecosystem.nix"),
            kind: CseCheckKind::ManifestMembership,
            action: EditAction::Skip {
                reason: "Adding to the manifest requires picking a class (gpu-desktop / tui-tool / dev-tool / mcp-server / server-app / ...) and deciding which profiles enable that class. Skipping — handle manually in nix/lib/ecosystem.nix.".into(),
            },
        })
    }
}

/// All remediators in deterministic order.
pub fn all_remediators() -> Vec<Box<dyn Remediator>> {
    vec![
        Box::new(ClaudeMdPointerRemediator),
        Box::new(HandRollRemediator),
        Box::new(LegacyModuleRemediator),
        Box::new(ManifestMembershipRemediator),
    ]
}

/// Apply a planned edit to disk. Idempotent — re-applying after the
/// fix is already in place is a no-op (the regex check sees the
/// pointer is present and the audit emits no violation).
pub fn apply_edit(edit: &PlannedEdit) -> Result<bool> {
    match &edit.action {
        EditAction::Skip { .. } => Ok(false),
        EditAction::InsertAfterFirstMatch {
            after_first_line_match,
            text,
        } => {
            let content = std::fs::read_to_string(&edit.path)?;
            let mut out = String::with_capacity(content.len() + text.len());
            let mut inserted = false;
            for line in content.lines() {
                out.push_str(line);
                out.push('\n');
                if !inserted && line == after_first_line_match {
                    out.push_str(text);
                    inserted = true;
                }
            }
            if !inserted {
                anyhow::bail!(
                    "expected line `{}` not found in {}",
                    after_first_line_match,
                    edit.path.display(),
                );
            }
            std::fs::write(&edit.path, out)?;
            Ok(true)
        }
    }
}

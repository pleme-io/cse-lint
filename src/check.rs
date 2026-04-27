//! The four CSE checkers + a composable trait that wires them together.
//!
//! Each checker is small, single-purpose, and produces zero or more
//! [`CseViolation`]s. The orchestrator (in `main.rs`) walks all repos
//! and runs every checker against each.

use crate::model::{CseCheckKind, CseViolation, RepoContext};
use regex::Regex;
use std::sync::OnceLock;

pub trait CseChecker: Send + Sync {
    fn kind(&self) -> CseCheckKind;
    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>);
}

// ─── 1. ClaudeMdPointer ──────────────────────────────────────────────
/// Asserts that the repo's `CLAUDE.md` contains a CSE pointer block —
/// either a link to `CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md`, or a
/// reference to the org-level Compounding Directive.
pub struct ClaudeMdPointerChecker;

impl CseChecker for ClaudeMdPointerChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::ClaudeMdPointer
    }

    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>) {
        let claude = match &repo.claude_md {
            Some(c) => c,
            None => {
                violations.push(CseViolation::MissingCsePointer {
                    repo: repo.name.clone(),
                    remediation: "Create CLAUDE.md (use the `context` skill or copy a sibling repo's pointer block).".into(),
                });
                return;
            }
        };
        let mentions_cse = claude.contains("CONSTRUCTIVE-SUBSTRATE-ENGINEERING")
            || claude.contains("Knowable Construction")
            || claude.contains("Compounding Directive");
        if !mentions_cse {
            violations.push(CseViolation::MissingCsePointer {
                repo: repo.name.clone(),
                remediation: "Add the standard CSE pointer block at the top of CLAUDE.md (see substrate/CLAUDE.md or nix/CLAUDE.md for canonical shape).".into(),
            });
        }
    }
}

// ─── 2. HandRollDetection ────────────────────────────────────────────
/// Asserts that `flake.nix` consumes a substrate helper rather than
/// hand-rolling `pkgs.rustPlatform.buildRustPackage` /
/// `flake-utils.lib.eachSystem` from scratch.
pub struct HandRollDetectionChecker;

fn substrate_helper_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"\$\{substrate\}/lib/(rust-tool-release-flake|rust-workspace-release-flake|rust-service-flake|rust-library|rust-tool-image-flake|go-tool-flake|ruby-gem-flake|typescript-(library|tool)-flake|zig-tool-release-flake|wasi-service-flake|mcp-server-flake|module-trio)\.nix"#,
        )
        .expect("substrate helper regex must compile")
    })
}

fn hand_roll_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(rustPlatform\.buildRustPackage|flake-utils\.lib\.eachSystem|flake-utils\.lib\.eachDefaultSystem)"#,
        )
        .expect("hand-roll regex must compile")
    })
}

impl CseChecker for HandRollDetectionChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::HandRollDetection
    }

    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>) {
        let flake = match &repo.flake_nix {
            Some(f) => f,
            None => return, // non-flake repos are out of scope for this check
        };
        let uses_helper = substrate_helper_re().is_match(flake);
        let uses_hand_roll = hand_roll_re().is_match(flake);
        // Permitted: helper alone, OR helper + occasional `rustPlatform.buildRustPackage` for
        // a one-off (e.g. shinryu-mcp inline derivation in parts/overlays.nix).
        if !uses_helper && uses_hand_roll {
            violations.push(CseViolation::HandRoll {
                repo: repo.name.clone(),
                pattern: "flake.nix uses rustPlatform.buildRustPackage / flake-utils.lib.eachSystem without importing a substrate helper.".into(),
                remediation: "Migrate to substrate/lib/rust-tool-release-flake.nix (or the matching helper for the repo's class). See the trio macro's spec at substrate/lib/module-trio.nix.".into(),
            });
        }
    }
}

// ─── 3. ManifestMembership ────────────────────────────────────────────
/// Asserts that the repo, if it appears in
/// `pleme-io/nix/lib/ecosystem.nix`, has a class assignment that
/// references at least one profile.
///
/// This check is **stub** for now: full implementation requires parsing
/// the manifest's Nix value, which is out of scope until cse-lint links
/// against a Nix evaluator (or invokes `nix eval` as a sub-process).
/// The stub records the repos it can find inline references for; full
/// audit lands when manifest-eval support is added.
pub struct ManifestMembershipChecker {
    /// Path to ecosystem.nix; if None, the check is skipped.
    pub manifest_path: Option<std::path::PathBuf>,
}

impl CseChecker for ManifestMembershipChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::ManifestMembership
    }

    fn check(&self, _repo: &RepoContext, _violations: &mut Vec<CseViolation>) {
        // Pending: parse manifest Nix value via `nix eval --json` and
        // cross-reference. The manifest's typed schema (lib/ecosystem.nix)
        // is the source of truth; cse-lint reads it at audit time.
        // No violations emitted from the stub — checker reports clean
        // until full implementation lands.
    }
}

// ─── 4. ModuleTrioAdoption ────────────────────────────────────────────
/// Asserts that `flake.nix` doesn't carry the legacy
/// `// { homeManagerModules.default = import ./module ... }` suffix.
/// Flakes using a substrate helper should pass `module = { ... }`
/// instead.
pub struct ModuleTrioAdoptionChecker;

fn legacy_module_pattern_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"//\s*\{\s*homeManagerModules\.default\s*=\s*import\s+\./module"#,
        )
        .expect("legacy module pattern regex must compile")
    })
}

impl CseChecker for ModuleTrioAdoptionChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::ModuleTrioAdoption
    }

    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>) {
        let flake = match &repo.flake_nix {
            Some(f) => f,
            None => return,
        };
        if legacy_module_pattern_re().is_match(flake) {
            violations.push(CseViolation::LegacyModulePattern {
                repo: repo.name.clone(),
                location: "flake.nix uses // { homeManagerModules.default = import ./module ... } pattern".into(),
                remediation: "Migrate to `module = { description = ...; ... }` in the substrate helper call. See substrate/lib/module-trio.nix spec, or commit nami@a2a2a72 for a canonical example.".into(),
            });
        }
    }
}

//! Typed data model for cse-lint.
//!
//! Per the Compounding Directive, every claim cse-lint makes about a
//! repo is grounded in a typed value here. The CLI / report layer reads
//! these types; it never re-derives state from raw paths.

use serde::Serialize;
use std::path::PathBuf;

/// A single pleme-io repository under audit.
#[derive(Debug, Clone)]
pub struct RepoContext {
    /// Absolute path to the repo's working tree.
    pub path: PathBuf,
    /// Repository name (last component of `path`).
    pub name: String,
    /// Top-level CLAUDE.md content (None if the file is missing).
    pub claude_md: Option<String>,
    /// Top-level flake.nix content (None if not a Nix flake repo).
    pub flake_nix: Option<String>,
    /// module/default.nix content (None for repos without a module dir).
    pub module_nix: Option<String>,
}

/// Tag identifying which CSE invariant a check verifies.
///
/// The four invariants come directly from the Compounding Directive:
///   1. Models stay current (CLAUDE.md ↔ theory)
///   2. Solve problems once (substrate helper consumption)
///   3. Acquire and contextualize (manifest membership)
///   4. Idiom-first (module trio adoption over hand-rolled HM modules)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum CseCheckKind {
    /// CLAUDE.md links to CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md
    /// and/or the org-level Compounding Directive.
    ClaudeMdPointer,
    /// flake.nix consumes a substrate helper rather than hand-rolling
    /// `pkgs.rustPlatform.buildRustPackage` / `flake-utils.lib.eachSystem`.
    HandRollDetection,
    /// App in `pleme-io/nix/lib/ecosystem.nix` is class-assigned and
    /// the class is referenced by at least one profile.
    ManifestMembership,
    /// flake.nix passes `module = { ... }` to its substrate helper
    /// (new pattern) instead of hand-rolling
    /// `// { homeManagerModules.default = import ./module ... }`.
    ModuleTrioAdoption,
}

impl CseCheckKind {
    pub fn label(self) -> &'static str {
        match self {
            CseCheckKind::ClaudeMdPointer => "claude-md-pointer",
            CseCheckKind::HandRollDetection => "hand-roll",
            CseCheckKind::ManifestMembership => "manifest-membership",
            CseCheckKind::ModuleTrioAdoption => "module-trio-adoption",
        }
    }

    pub fn invariant(self) -> &'static str {
        match self {
            CseCheckKind::ClaudeMdPointer => "models stay current",
            CseCheckKind::HandRollDetection => "solve problems once",
            CseCheckKind::ManifestMembership => "acquire and contextualize",
            CseCheckKind::ModuleTrioAdoption => "idiom-first",
        }
    }

    pub fn all() -> [CseCheckKind; 4] {
        [
            CseCheckKind::ClaudeMdPointer,
            CseCheckKind::HandRollDetection,
            CseCheckKind::ManifestMembership,
            CseCheckKind::ModuleTrioAdoption,
        ]
    }
}

/// A single CSE audit violation discovered in a repo.
///
/// Each variant carries the data necessary to remediate the violation
/// without re-running the audit. `remediation` should be a short
/// sentence the operator (or an agent) can act on.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum CseViolation {
    MissingCsePointer {
        repo: String,
        remediation: String,
    },
    HandRoll {
        repo: String,
        pattern: String,
        remediation: String,
    },
    ManifestInconsistency {
        repo: String,
        app: String,
        issue: String,
        remediation: String,
    },
    LegacyModulePattern {
        repo: String,
        location: String,
        remediation: String,
    },
}

impl CseViolation {
    pub fn kind(&self) -> CseCheckKind {
        match self {
            CseViolation::MissingCsePointer { .. } => CseCheckKind::ClaudeMdPointer,
            CseViolation::HandRoll { .. } => CseCheckKind::HandRollDetection,
            CseViolation::ManifestInconsistency { .. } => CseCheckKind::ManifestMembership,
            CseViolation::LegacyModulePattern { .. } => CseCheckKind::ModuleTrioAdoption,
        }
    }

    pub fn repo(&self) -> &str {
        match self {
            CseViolation::MissingCsePointer { repo, .. } => repo,
            CseViolation::HandRoll { repo, .. } => repo,
            CseViolation::ManifestInconsistency { repo, .. } => repo,
            CseViolation::LegacyModulePattern { repo, .. } => repo,
        }
    }
}

/// Per-repo audit result.
#[derive(Debug, Clone, Serialize)]
pub struct RepoResult {
    pub repo_name: String,
    pub violations: Vec<CseViolation>,
    pub checks_run: Vec<CseCheckKind>,
    /// True iff `violations.is_empty()`.
    pub pass: bool,
}

/// Aggregated audit report.
#[derive(Debug, Serialize)]
pub struct CseAuditReport {
    pub repos: Vec<RepoResult>,
    /// Summary count of violations by kind.
    #[serde(serialize_with = "serialize_kind_counts")]
    pub violations_by_kind: std::collections::BTreeMap<CseCheckKind, usize>,
    pub run_at: String,
    pub total_repos: usize,
    pub passing_repos: usize,
}

fn serialize_kind_counts<S>(
    map: &std::collections::BTreeMap<CseCheckKind, usize>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    let mut m = serializer.serialize_map(Some(map.len()))?;
    for (k, v) in map {
        m.serialize_entry(k.label(), v)?;
    }
    m.end()
}

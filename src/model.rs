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
    /// Top-level caixa.lisp content (None if the repo isn't caixa-native).
    pub caixa_lisp: Option<String>,
    /// Top-level Cargo.toml content (None for non-Rust repos). Used by
    /// the theme-architecture invariants (GuiAppConsumesIshou etc.) to
    /// inspect declared dependencies without re-reading from disk per
    /// checker.
    pub cargo_toml: Option<String>,
    /// Top-level `Cargo.build-spec.json` content. Read by the
    /// build-spec invariants (BuildSpecCanonicalUrl, BuildSpecSchemaVersion)
    /// that gate fleet drift back into the substrate's value-management
    /// space.
    pub cargo_build_spec_json: Option<String>,
}

/// Tag identifying which CSE invariant a check verifies.
///
/// The invariants come directly from the Compounding Directive:
///   1. Models stay current (CLAUDE.md ↔ theory)
///   2. Solve problems once (substrate helper consumption)
///   3. Acquire and contextualize (manifest membership)
///   4. Idiom-first (module trio adoption over hand-rolled HM modules)
///   5. Promises hold mechanically (deployment coverage)
///   6. Generation over composition (caixa-native authoring)
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
    /// A repo whose flake.nix declares `deploy = { cluster = "...";`
    /// (i.e. uses substrate's wasi-service-flux-flake.nix) has a
    /// matching FluxCD bundle directory at
    /// `pleme-io/k8s/clusters/<cluster>/services/<name>/`.
    DeploymentCoverage,
    /// The repo declares itself as a caixa via a `caixa.lisp` at the root.
    /// V0 is informational (severity = warn) — flips to `--strict` once
    /// the migration window completes. Drives fleet adoption of caixa as
    /// the primitive (per theory/META-FRAMEWORK.md §I).
    CaixaNaivete,
    /// Any Rust crate that depends on `garasu` or `madori` (i.e. is a
    /// pleme-io GUI app on the substrate's GPU stack) MUST also depend
    /// on `ishou-tokens`. The theme architecture
    /// (theory/THEME-ARCHITECTURE.md) makes gamma-correct colour
    /// construction structurally impossible without ishou-tokens'
    /// typed Srgb → Linear → wgpu::Color path.
    GuiAppConsumesIshou,
    /// The only file in the pleme-io source tree carrying a Nord
    /// palette source is `ishou-tokens/src/color.rs` (plus its tests).
    /// Any other file with a hardcoded `#2e3440` / `polar_night_0` /
    /// `aurora_red` literal is fleet drift; the theme architecture
    /// makes ishou the single source for foreign apps (via the stylix
    /// base16 render) and pleme-io GUI apps (via the typed Cargo dep).
    NoForeignNordSource,
    /// Any pleme-io GPU app (Cargo.toml depends on garasu / madori) MUST
    /// ship an MCP-backed headless mode the way mado does — see
    /// `pleme-io/theory/HEADLESS-INTROSPECTION.md`. The audit checks
    /// for: an `mcp` subcommand declared in `Cli` / `SubCmd`, a
    /// `tests/scenarios/` directory, and a `scenario` module. Missing
    /// any of these is fleet drift — every visual bug becomes an
    /// operator-only screenshot until the headless surface exists.
    GpuAppHeadlessMode,
    /// Any binary that runs an MCP server (rmcp / kaname) MUST init
    /// tracing to stderr in its `mcp` subcommand so stdout stays clean
    /// for the JSON-RPC framing. Detected by grepping the binary's
    /// main.rs for `init_tracing_to_stderr` adjacent to the MCP
    /// dispatch site.
    McpStdoutClean,
    /// Any pleme-io GPU app crate MUST ship at least one
    /// `*.scenario.yaml` in `tests/scenarios/`. Missing a corpus
    /// means there's nothing CI-gating the app's behaviour at the
    /// cell/state level — operator-only screenshots are not
    /// regression tests.
    ScenarioCorpusPresent,
    /// Every committed `Cargo.build-spec.json` MUST emit registry
    /// URLs in the canonical `https://static.crates.io/crates/...`
    /// form. The `/api/v1/.../download` redirect endpoint is rate-
    /// limited and now 403's against nixpkgs' UA-less fetchurl.
    /// Substrate has a transitional Nix-side rewrite; the source-of-
    /// truth gate is here.
    BuildSpecCanonicalUrl,
    /// Every committed `Cargo.build-spec.json` MUST carry the current
    /// gen-cargo `SCHEMA_VERSION`. Older specs are missing the typed
    /// `build_rust_crate_args` field; substrate's lockfile-builder has
    /// transitional `legacyArgs` fallback, sunsetting at M6.
    BuildSpecSchemaVersion,
    /// Per the prime directive (consumer flake = 4 lines), no
    /// consumer `flake.nix` should directly `import "${substrate}/lib
    /// /build/rust/lockfile-builder.nix"`. Use `substrate.rust.tool`
    /// or `substrate.rust.workspace` instead.
    NoLockfileBuilderDirectImport,
}

impl CseCheckKind {
    pub fn label(self) -> &'static str {
        match self {
            CseCheckKind::ClaudeMdPointer => "claude-md-pointer",
            CseCheckKind::HandRollDetection => "hand-roll",
            CseCheckKind::ManifestMembership => "manifest-membership",
            CseCheckKind::ModuleTrioAdoption => "module-trio-adoption",
            CseCheckKind::DeploymentCoverage => "deployment-coverage",
            CseCheckKind::CaixaNaivete => "caixa-naivete",
            CseCheckKind::GuiAppConsumesIshou => "gui-app-consumes-ishou",
            CseCheckKind::NoForeignNordSource => "no-foreign-nord-source",
            CseCheckKind::GpuAppHeadlessMode => "gpu-app-headless-mode",
            CseCheckKind::McpStdoutClean => "mcp-stdout-clean",
            CseCheckKind::ScenarioCorpusPresent => "scenario-corpus-present",
            CseCheckKind::BuildSpecCanonicalUrl => "build-spec-canonical-url",
            CseCheckKind::BuildSpecSchemaVersion => "build-spec-schema-version",
            CseCheckKind::NoLockfileBuilderDirectImport => "no-lockfile-builder-direct-import",
        }
    }

    pub fn invariant(self) -> &'static str {
        match self {
            CseCheckKind::ClaudeMdPointer => "models stay current",
            CseCheckKind::HandRollDetection => "solve problems once",
            CseCheckKind::ManifestMembership => "acquire and contextualize",
            CseCheckKind::ModuleTrioAdoption => "idiom-first",
            CseCheckKind::DeploymentCoverage => "promises hold mechanically",
            CseCheckKind::CaixaNaivete => "generation over composition",
            CseCheckKind::GuiAppConsumesIshou => "one typed theme primitive",
            CseCheckKind::NoForeignNordSource => "one typed theme primitive",
            CseCheckKind::GpuAppHeadlessMode => "every GPU app provably self-diagnoses",
            CseCheckKind::McpStdoutClean => "every MCP binary keeps stdout clean",
            CseCheckKind::ScenarioCorpusPresent => "every fix lands with its regression test",
            CseCheckKind::BuildSpecCanonicalUrl => "rust owns value computation",
            CseCheckKind::BuildSpecSchemaVersion => "single source of truth",
            CseCheckKind::NoLockfileBuilderDirectImport => "consumer flake is 4 lines",
        }
    }

    pub fn all() -> [CseCheckKind; 14] {
        [
            CseCheckKind::ClaudeMdPointer,
            CseCheckKind::HandRollDetection,
            CseCheckKind::ManifestMembership,
            CseCheckKind::ModuleTrioAdoption,
            CseCheckKind::DeploymentCoverage,
            CseCheckKind::CaixaNaivete,
            CseCheckKind::GuiAppConsumesIshou,
            CseCheckKind::NoForeignNordSource,
            CseCheckKind::GpuAppHeadlessMode,
            CseCheckKind::McpStdoutClean,
            CseCheckKind::ScenarioCorpusPresent,
            CseCheckKind::BuildSpecCanonicalUrl,
            CseCheckKind::BuildSpecSchemaVersion,
            CseCheckKind::NoLockfileBuilderDirectImport,
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
    /// flake.nix declares `deploy = { cluster = "..." }` but the
    /// expected FluxCD bundle directory at
    /// `k8s/clusters/<cluster>/services/<name>/` is absent or empty.
    MissingDeployBundle {
        repo: String,
        cluster: String,
        expected_path: String,
        remediation: String,
    },
    /// The repo lacks a `caixa.lisp` at its root. V0 is informational —
    /// flips to a hard failure under `--strict` once the migration window
    /// closes.
    MissingCaixaManifest {
        repo: String,
        remediation: String,
    },
    /// The repo's Cargo.toml depends on `garasu` and/or `madori` (i.e. is
    /// a pleme-io GUI app) but doesn't depend on `ishou-tokens`. The
    /// theme architecture mandates `ishou-tokens` as the typed colour-
    /// space primitive for every fleet GUI consumer.
    MissingIshouTokensDep {
        repo: String,
        gpu_dep: String,
        remediation: String,
    },
    /// The repo carries a Nord palette source file outside ishou-tokens —
    /// e.g. a local `themes/nord/colors.nix`, a hardcoded `#2e3440`
    /// constant in a non-test file, an inline base16 yaml. The fleet's
    /// one Nord lives in `ishou-tokens/src/color.rs`.
    ForeignNordSource {
        repo: String,
        relative_path: String,
        remediation: String,
    },
    /// A GPU app (Cargo.toml depends on garasu / madori) is missing
    /// one of the canonical headless-introspection primitives —
    /// `mcp` subcommand, `tests/scenarios/`, or a `scenario` module.
    /// See `theory/HEADLESS-INTROSPECTION.md` for the full pattern.
    MissingHeadlessPrimitive {
        repo: String,
        missing: String,
        remediation: String,
    },
    /// A binary that registers an rmcp / kaname MCP server doesn't
    /// init tracing to stderr — stdout pollution breaks JSON-RPC
    /// framing in MCP mode. See `shidou::init_tracing_to_stderr`.
    McpStdoutPolluted {
        repo: String,
        remediation: String,
    },
    /// A GPU app has a `tests/scenarios/` directory but it contains
    /// no `*.scenario.yaml` files. The corpus is the CI-gated
    /// substrate of provable behaviour — empty corpus = no proof.
    EmptyScenarioCorpus {
        repo: String,
        remediation: String,
    },
    /// `Cargo.build-spec.json` carries one or more `crates.io/api/v1/`
    /// URLs. gen-cargo emits canonical `static.crates.io` URLs since
    /// 70774a2; older specs MUST be regenerated.
    BuildSpecApiUrl {
        repo: String,
        count: usize,
        remediation: String,
    },
    /// `Cargo.build-spec.json` schema version is below the current
    /// gen-cargo `SCHEMA_VERSION`. Substrate's lockfile-builder is
    /// transitional today, hard-gating at M6.
    BuildSpecStaleSchema {
        repo: String,
        found: u32,
        expected: u32,
        remediation: String,
    },
    /// `flake.nix` imports `lockfile-builder.nix` directly instead of
    /// going through the canonical `substrate.rust.tool` /
    /// `substrate.rust.workspace` 4-line shape. Forces the consumer
    /// flake to be invariantly minimal.
    LockfileBuilderDirectImport {
        repo: String,
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
            CseViolation::MissingDeployBundle { .. } => CseCheckKind::DeploymentCoverage,
            CseViolation::MissingCaixaManifest { .. } => CseCheckKind::CaixaNaivete,
            CseViolation::MissingIshouTokensDep { .. } => CseCheckKind::GuiAppConsumesIshou,
            CseViolation::ForeignNordSource { .. } => CseCheckKind::NoForeignNordSource,
            CseViolation::MissingHeadlessPrimitive { .. } => CseCheckKind::GpuAppHeadlessMode,
            CseViolation::McpStdoutPolluted { .. } => CseCheckKind::McpStdoutClean,
            CseViolation::EmptyScenarioCorpus { .. } => CseCheckKind::ScenarioCorpusPresent,
            CseViolation::BuildSpecApiUrl { .. } => CseCheckKind::BuildSpecCanonicalUrl,
            CseViolation::BuildSpecStaleSchema { .. } => CseCheckKind::BuildSpecSchemaVersion,
            CseViolation::LockfileBuilderDirectImport { .. } => {
                CseCheckKind::NoLockfileBuilderDirectImport
            }
        }
    }

    pub fn repo(&self) -> &str {
        match self {
            CseViolation::MissingCsePointer { repo, .. } => repo,
            CseViolation::HandRoll { repo, .. } => repo,
            CseViolation::ManifestInconsistency { repo, .. } => repo,
            CseViolation::LegacyModulePattern { repo, .. } => repo,
            CseViolation::MissingDeployBundle { repo, .. } => repo,
            CseViolation::MissingCaixaManifest { repo, .. } => repo,
            CseViolation::MissingIshouTokensDep { repo, .. } => repo,
            CseViolation::ForeignNordSource { repo, .. } => repo,
            CseViolation::MissingHeadlessPrimitive { repo, .. } => repo,
            CseViolation::McpStdoutPolluted { repo, .. } => repo,
            CseViolation::EmptyScenarioCorpus { repo, .. } => repo,
            CseViolation::BuildSpecApiUrl { repo, .. } => repo,
            CseViolation::BuildSpecStaleSchema { repo, .. } => repo,
            CseViolation::LockfileBuilderDirectImport { repo, .. } => repo,
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

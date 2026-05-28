//! The CSE checkers + a composable trait that wires them together.
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
/// Implementation: invokes `nix eval --json` on the manifest's
/// `resolved` (typed apps + classes), then cross-references each app's
/// class against the classes table's `profiles` list. Apps with a
/// class that has zero profile members get flagged — they're listed in
/// the manifest but never enabled anywhere.
///
/// We don't flag "this repo is missing from the manifest" because the
/// manifest is intentionally a curated subset (the trio-migrated apps),
/// not the entire fleet.
pub struct ManifestMembershipChecker {
    /// Path to ecosystem.nix; if None, the check is skipped.
    pub manifest_path: Option<std::path::PathBuf>,
    /// Eagerly-loaded manifest content. Populated by `load`.
    pub loaded: std::sync::OnceLock<ManifestSnapshot>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ManifestSnapshot {
    /// app-name → resolved-app shape (we only keep what we audit).
    #[serde(default)]
    pub resolved: std::collections::HashMap<String, ResolvedApp>,
    /// class-name → class shape.
    #[serde(default)]
    pub classes: std::collections::HashMap<String, ResolvedClass>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResolvedApp {
    pub class: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default, rename = "optionName")]
    pub option_name: Option<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResolvedClass {
    #[serde(default)]
    pub profiles: Vec<String>,
    /// Apps in this class are listed for knowledge-graph / audit
    /// completeness; empty profiles[] is intentional, not a violation.
    #[serde(default, rename = "auditOnly")]
    pub audit_only: bool,
}

impl ManifestMembershipChecker {
    pub fn new(manifest_path: Option<std::path::PathBuf>) -> Self {
        Self {
            manifest_path,
            loaded: std::sync::OnceLock::new(),
        }
    }

    /// Invoke `nix eval --json` against the manifest. Returns None on
    /// any error (manifest absent, nix unavailable, eval failure) —
    /// the audit then skips manifest-membership rather than failing.
    fn load_snapshot(&self) -> Option<&ManifestSnapshot> {
        if let Some(snap) = self.loaded.get() {
            return Some(snap);
        }
        let path = self.manifest_path.as_ref()?;
        let workspace_root = path.parent()?.parent()?.parent()?;
        // We import the manifest as a Nix value. The `inputs` arg is
        // tricky — we'd need the workspace's flake. Shortcut: tell nix
        // to eval the manifest as a module, supplying a fake `inputs`
        // attrset whose presence we don't actually check (the manifest
        // looks up `inputs.<app>.homeManagerModules` only when the
        // helper functions are CALLED; the data structure itself is
        // pure).
        let expr = format!(
            r#"
              let
                lib = (import <nixpkgs> {{}}).lib;
                # Provide a stub inputs that returns an empty attrset
                # for any flake input lookup. The manifest's resolution
                # functions don't dereference inputs unless asked, so
                # we can pull just the data shape.
                eco = import {} {{ inputs = {{}}; lib = lib; }};
              in {{
                resolved = lib.mapAttrs (_: app: {{ class = app.class or null; }}) eco.resolved;
                classes = lib.mapAttrs (_: cls: {{
                  profiles = cls.profiles or [];
                  auditOnly = cls.auditOnly or false;
                }}) eco.classes;
              }}
            "#,
            path.display(),
        );
        let output = std::process::Command::new("nix")
            .args(&["eval", "--impure", "--json", "--expr", &expr])
            .current_dir(workspace_root)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8(output.stdout).ok()?;
        let snap: ManifestSnapshot = serde_json::from_str(&stdout).ok()?;
        let _ = self.loaded.set(snap);
        self.loaded.get()
    }
}

impl CseChecker for ManifestMembershipChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::ManifestMembership
    }

    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>) {
        let snap = match self.load_snapshot() {
            Some(s) => s,
            None => return, // manifest not available; skip silently
        };
        let app = match snap.resolved.get(&repo.name) {
            Some(a) => a,
            None => return, // not in manifest; that's fine — manifest is curated
        };
        let class_name = match &app.class {
            Some(c) => c,
            None => {
                violations.push(CseViolation::ManifestInconsistency {
                    repo: repo.name.clone(),
                    app: repo.name.clone(),
                    issue: "in manifest but no class assigned".into(),
                    remediation: "Add `class = \"<class-name>\"` to the entry in pleme-io/nix/lib/ecosystem.nix.".into(),
                });
                return;
            }
        };
        let class = match snap.classes.get(class_name) {
            Some(c) => c,
            None => {
                violations.push(CseViolation::ManifestInconsistency {
                    repo: repo.name.clone(),
                    app: repo.name.clone(),
                    issue: format!("class `{}` is not defined", class_name),
                    remediation: format!("Add `\"{}\" = {{ profiles = [...]; }};` to the classes attrset in pleme-io/nix/lib/ecosystem.nix.", class_name),
                });
                return;
            }
        };
        if class.profiles.is_empty() && !class.audit_only {
            violations.push(CseViolation::ManifestInconsistency {
                repo: repo.name.clone(),
                app: repo.name.clone(),
                issue: format!("class `{}` has no profile members — app is loaded but never auto-enabled", class_name),
                remediation: format!("Either add this class to a profile's enable set in pleme-io/nix/lib/ecosystem.nix, or move the app to an existing class with profile members. If empty profiles[] is intentional, set `auditOnly = true;` on the class so cse-lint stops flagging it."),
            });
        }
    }
}

// ─── 5. DeploymentCoverage ────────────────────────────────────────────
/// Asserts that any flake.nix declaring `deploy = { cluster = "..."; }`
/// (i.e. consuming `wasi-service-flux-flake.nix`) has a corresponding
/// FluxCD bundle directory in `pleme-io/k8s/clusters/<cluster>/services/<name>/`.
///
/// The audit walks the repo's flake.nix, extracts the `deploy.cluster`
/// value via regex, then verifies the expected on-disk path exists in
/// the workspace's `k8s/clusters/<cluster>/services/<repo-name>/`
/// directory and contains at least one YAML file.
pub struct DeploymentCoverageChecker {
    /// Path to the workspace root containing both repos and the k8s
    /// directory. If None, the check is skipped silently.
    pub workspace_root: Option<std::path::PathBuf>,
}

fn deploy_cluster_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"deploy\s*=\s*\{[^}]*?cluster\s*=\s*"([^"]+)""#)
            .expect("deploy.cluster regex must compile")
    })
}

impl CseChecker for DeploymentCoverageChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::DeploymentCoverage
    }

    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>) {
        let root = match &self.workspace_root {
            Some(r) => r,
            None => return,
        };
        let flake = match &repo.flake_nix {
            Some(f) => f,
            None => return,
        };
        // Find the first deploy.cluster ref. If none, the repo doesn't
        // declare a deploy block — out of scope for this check.
        let caps = match deploy_cluster_re().captures(flake) {
            Some(c) => c,
            None => return,
        };
        let cluster = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        if cluster.is_empty() {
            return;
        }
        let expected = root
            .join("k8s")
            .join("clusters")
            .join(cluster)
            .join("services")
            .join(&repo.name);
        let bundle_present = expected.is_dir()
            && std::fs::read_dir(&expected)
                .map(|d| d.flatten().any(|e| {
                    e.path()
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s == "yaml" || s == "yml")
                        .unwrap_or(false)
                }))
                .unwrap_or(false);
        if !bundle_present {
            violations.push(CseViolation::MissingDeployBundle {
                repo: repo.name.clone(),
                cluster: cluster.to_string(),
                expected_path: expected.display().to_string(),
                remediation: format!(
                    "Run `cd ~/code/github/pleme-io/{} && nix run .#render-deploy` \
                     to materialize the bundle, then commit the resulting \
                     directory to the k8s repo. (Substrate primitive: \
                     wasi-service-flux-flake.nix.)",
                    repo.name,
                ),
            });
        }
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


// ─── 6. CaixaNaivete ──────────────────────────────────────────────────
/// Asserts that the repo declares itself as a caixa via a top-level
/// `caixa.lisp` (per `theory/META-FRAMEWORK.md` §I — caixa is the
/// canonical Layer-0 source primitive).
///
/// V0 flags every repo without `caixa.lisp` as informational. Eventually
/// `--strict` makes it a hard failure for repos that should be caixa-native
/// (Servico/Biblioteca/Binario shapes — i.e. anything authored, as opposed
/// to in-repo Helm charts and operator manifests).
///
/// The driver in `main.rs` runs only one checker per repo per kind, and
/// the violation message points at the canonical migration recipe (`feira
/// init` from caixa-feira, or copying hello-rio's caixa.lisp template).
pub struct CaixaNaiveteChecker;

impl CseChecker for CaixaNaiveteChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::CaixaNaivete
    }

    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>) {
        if repo.caixa_lisp.is_some() {
            return;
        }
        // Skip well-known infra/meta repos that are *deliberately* not
        // caixas — the org-wide GitOps tree, the theory docs, the
        // pangea-architectures workspace, the catalog repos, etc. These
        // get a free pass; everything else is fair game.
        const EXEMPT: &[&str] = &[
            "k8s",
            "theory",
            "nix",
            "repo-forge",
            "pangea-architectures",
            "blackmatter",
            "blackmatter-shell",
            "blackmatter-nvim",
            "blackmatter-desktop",
            "blackmatter-claude",
            "blackmatter-pleme",
            "blackmatter-kubernetes",
            "blackmatter-secrets",
            "blackmatter-ghostty",
            "blackmatter-macos",
            "blackmatter-tailscale",
            "blackmatter-anvil",
            "blackmatter-cursor",
            "blackmatter-movie",
            "blackmatter-security",
            "blackmatter-services",
            "blackmatter-tend",
            "blackmatter-akeyless",
            "blackmatter-atlassian",
            "blackmatter-zig",
            "blackmatter-go",
            "blackmatter-opencode",
            "blackmatter-ayatsuri",
            "kindling-profiles",
            "helmworks",
            "lareira-charts",
        ];
        if EXEMPT.iter().any(|n| *n == repo.name.as_str()) {
            return;
        }
        violations.push(CseViolation::MissingCaixaManifest {
            repo: repo.name.clone(),
            remediation: "Author a `caixa.lisp` at the repo root via \
                `nix run github:pleme-io/caixa#feira -- init <name>` (or \
                copy hello-rio/caixa.lisp as a template). caixa is the \
                canonical Layer-0 source primitive — see \
                theory/META-FRAMEWORK.md §I."
                .into(),
        });
    }
}

// ─── 7. GuiAppConsumesIshou ──────────────────────────────────────────
/// Asserts that any Rust crate consuming the substrate's GPU stack
/// (`garasu` and/or `madori`) also consumes `ishou-tokens` — the
/// fleet's typed colour-space primitive. Without `ishou-tokens` the
/// app must hand-roll an `Srgb → Linear` conversion (or, worse, write
/// raw sRGB into a wgpu surface and accept the gamma confusion), both
/// of which the theme architecture eliminates by construction.
///
/// See `pleme-io/theory/THEME-ARCHITECTURE.md`.
pub struct GuiAppConsumesIshouChecker;

impl CseChecker for GuiAppConsumesIshouChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::GuiAppConsumesIshou
    }

    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>) {
        let Some(cargo) = repo.cargo_toml.as_ref() else {
            return;
        };
        // Exempt: ishou (can't depend on itself) and the substrate
        // libraries `garasu`/`madori`/`egaku`/`irodzuki`/`irodori` —
        // they sit *below* ishou-tokens in the dep graph and would
        // form a cycle. Same for ishou-tokens' own siblings.
        const EXEMPT: &[&str] = &[
            "ishou",
            "garasu",
            "madori",
            "egaku",
            "irodzuki",
            "irodori",
            "shikumi",
            "kaname",
            "hasami",
            "tsunagu",
            "tsuuchi",
            "soushi",
            "awase",
        ];
        if EXEMPT.iter().any(|n| *n == repo.name.as_str()) {
            return;
        }
        // Heuristic: a "GUI app on the substrate's GPU stack" depends
        // on garasu or madori as a Cargo dep. `crate-name = "…"` lines
        // in [dependencies] / [dev-dependencies] sections (and the
        // `garasu = { …` shorthand) all contain `garasu` / `madori`.
        // The check is a substring match on the raw Cargo.toml — TOML
        // parse-then-inspect would be more precise but no false
        // positives in practice across the fleet.
        let depends_on_garasu = cargo.contains("garasu");
        let depends_on_madori = cargo.contains("madori");
        if !(depends_on_garasu || depends_on_madori) {
            return;
        }
        if cargo.contains("ishou-tokens") || cargo.contains("ishou_tokens") {
            return;
        }
        let gpu_dep = if depends_on_garasu { "garasu" } else { "madori" };
        violations.push(CseViolation::MissingIshouTokensDep {
            repo: repo.name.clone(),
            gpu_dep: gpu_dep.into(),
            remediation: "Add `ishou-tokens = { git = \
                \"https://github.com/pleme-io/ishou\", features = [\"wgpu\"] }` \
                to Cargo.toml's [dependencies] and route every \
                `wgpu::Color` construction through \
                `ishou_tokens::Srgb::to_linear`. See \
                pleme-io/theory/THEME-ARCHITECTURE.md."
                .into(),
        });
    }
}

// ─── 8. NoForeignNordSource ──────────────────────────────────────────
/// Asserts that no file in the repo source tree carries a hardcoded
/// Nord palette source outside `ishou-tokens`. The architecture mandates
/// one Nord: the ishou Nord. Local copies — `themes/nord/colors.nix`,
/// inline `#2e3440` literals in render code, base16-shaped YAML fixtures
/// not under `tests/` — all drift the fleet.
///
/// Detection is structural: a tracked file path that *names* Nord
/// outside the test/fixture pattern. Inline literal scans are
/// intentionally **not** part of V0 because legitimate uses exist
/// (transient hex tests, docs that quote the canonical palette). The
/// structural rule (no `themes/nord/` directories outside ishou) is
/// the load-bearing one — it's how mado, ayatsuri, blackmatter-mado
/// historically each grew their own copy. See
/// `pleme-io/theory/THEME-ARCHITECTURE.md`.
pub struct NoForeignNordSourceChecker;

impl CseChecker for NoForeignNordSourceChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::NoForeignNordSource
    }

    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>) {
        // Exempt ishou (the source) and theory (which quotes the
        // palette for documentation). Everywhere else fair game.
        if repo.name == "ishou" || repo.name == "theory" || repo.name == "irodori" {
            return;
        }
        // Walk a few well-known "local theme" paths. Cheap: O(handful
        // of stat calls) per repo. Add patterns as the fleet surfaces
        // them; the existing set covers every drift we've already
        // observed.
        const SUSPECTS: &[&str] = &[
            "module/themes/nord/colors.nix",
            "module/themes/nord.yaml",
            "themes/nord/colors.nix",
            "themes/nord.yaml",
            "src/themes/nord.rs",
            "assets/nord.yaml",
        ];
        for suspect in SUSPECTS {
            let p = repo.path.join(suspect);
            if !p.exists() {
                continue;
            }
            // Skip files that mark themselves as ishou-derived
            // mirrors (header comment `Generated by ishou-render`). A
            // wrapper that ships a byte-derived projection of the
            // single source isn't drift — every regeneration produces
            // identical bytes from the same TokenSet, and an edit to
            // ishou-tokens' palette re-renders these files on the
            // next pleme-io/ishou build. Marker pattern matches the
            // header ishou-render::{stylix,nix,glsl,…} emit.
            let is_derived = std::fs::read_to_string(&p)
                .map(|c| c.contains("Generated by ishou-render"))
                .unwrap_or(false);
            if is_derived {
                continue;
            }
            violations.push(CseViolation::ForeignNordSource {
                repo: repo.name.clone(),
                relative_path: (*suspect).to_string(),
                remediation: "Delete the local Nord file and consume the \
                    fleet's canonical palette via \
                    `inputs.ishou.packages.${system}.stylix-base16-nord-dark` \
                    (foreign-app HM modules), \
                    `import inputs.ishou.packages.${system}.nord-palette-nix` \
                    (Nix consumers), or \
                    `ishou_tokens::ColorPalette::pleme()` \
                    (Rust GUI apps). Alternatively, regenerate the local file \
                    via `nix run github:pleme-io/ishou#nix > <path>` — \
                    the rendered output carries an `ishou-render` header \
                    this checker recognises as a byte-derived mirror. See \
                    pleme-io/theory/THEME-ARCHITECTURE.md."
                    .into(),
            });
        }
    }
}

// ─── 9. GpuAppHeadlessMode ────────────────────────────────────────────
/// Every pleme-io GPU app (Cargo.toml depends on `garasu` and/or
/// `madori`) must ship the canonical headless-introspection surface:
///
/// - A `Mcp` subcommand in the binary's CLI enum, AND
/// - A `tests/scenarios/` directory, AND
/// - A `scenario` Rust module declared at the binary root.
///
/// Each missing primitive emits one violation so the operator sees
/// every gap at once. See `theory/HEADLESS-INTROSPECTION.md` for the
/// pattern and `mado` for the reference implementation.
pub struct GpuAppHeadlessModeChecker;

impl CseChecker for GpuAppHeadlessModeChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::GpuAppHeadlessMode
    }

    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>) {
        let Some(cargo) = repo.cargo_toml.as_ref() else {
            return;
        };
        // Exempt: the substrate libraries that sit *below* GPU apps in
        // the dep graph, plus repos that aren't binaries.
        const EXEMPT: &[&str] = &[
            "garasu",
            "madori",
            "egaku",
            "irodzuki",
            "irodori",
            "shikumi",
            "kaname",
            "ishou",
        ];
        if EXEMPT.iter().any(|n| *n == repo.name.as_str()) {
            return;
        }
        // GPU app heuristic: depends on garasu or madori. Same shape
        // as the GuiAppConsumesIshou check — when both fire on the
        // same repo, the operator gets the full diagnosis.
        let is_gpu_app = cargo.contains("garasu") || cargo.contains("madori");
        if !is_gpu_app {
            return;
        }

        // Check 1: scenario module declared at binary root.
        let main_rs = repo.path.join("src/main.rs");
        let main_has_scenario_mod = std::fs::read_to_string(&main_rs)
            .map(|s| s.lines().any(|l| l.trim().starts_with("mod scenario")))
            .unwrap_or(false);
        if !main_has_scenario_mod {
            violations.push(CseViolation::MissingHeadlessPrimitive {
                repo: repo.name.clone(),
                missing: "scenario module".into(),
                remediation: "Add `mod scenario;` to src/main.rs and \
                    port the scenario harness from \
                    pleme-io/mado/src/scenario.rs. See \
                    theory/HEADLESS-INTROSPECTION.md §4."
                    .into(),
            });
        }

        // Check 2: tests/scenarios directory.
        let scenarios_dir = repo.path.join("tests/scenarios");
        if !scenarios_dir.is_dir() {
            violations.push(CseViolation::MissingHeadlessPrimitive {
                repo: repo.name.clone(),
                missing: "tests/scenarios/ directory".into(),
                remediation: "Create tests/scenarios/ and add at least \
                    one *.scenario.yaml. See \
                    pleme-io/mado/tests/scenarios/ for the canonical \
                    layout."
                    .into(),
            });
        }

        // Check 3: MCP subcommand. Heuristic — main.rs declares a
        // subcommand whose name is `Mcp` (matches mado / nascent
        // fleet pattern).
        let main_has_mcp = std::fs::read_to_string(&main_rs)
            .map(|s| s.contains("Mcp") && s.contains("SubCmd"))
            .unwrap_or(false);
        if !main_has_mcp {
            violations.push(CseViolation::MissingHeadlessPrimitive {
                repo: repo.name.clone(),
                missing: "mcp subcommand".into(),
                remediation: "Add a `Mcp` variant to your CLI's `SubCmd` \
                    enum and dispatch to an in-process kaname MCP \
                    server. See pleme-io/mado/src/main.rs."
                    .into(),
            });
        }
    }
}

// ─── 10. McpStdoutClean ───────────────────────────────────────────────
/// Any binary that registers an rmcp / kaname MCP server must route
/// tracing to **stderr** so stdout stays clean for JSON-RPC framing.
/// Detection: a repo's Cargo.toml mentions rmcp / kaname (it's an MCP
/// host) AND main.rs lacks `init_tracing_to_stderr` (it's polluting
/// stdout).
pub struct McpStdoutCleanChecker;

impl CseChecker for McpStdoutCleanChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::McpStdoutClean
    }

    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>) {
        let Some(cargo) = repo.cargo_toml.as_ref() else {
            return;
        };
        // Exempt the canonical kaname / shidou libraries themselves —
        // they define the primitives, they don't consume them.
        if matches!(repo.name.as_str(), "kaname" | "shidou") {
            return;
        }
        let hosts_mcp = cargo.contains("rmcp") || cargo.contains("kaname");
        if !hosts_mcp {
            return;
        }
        let main_rs = repo.path.join("src/main.rs");
        let main = match std::fs::read_to_string(&main_rs) {
            Ok(s) => s,
            Err(_) => return,
        };
        if main.contains("init_tracing_to_stderr") {
            return;
        }
        violations.push(CseViolation::McpStdoutPolluted {
            repo: repo.name.clone(),
            remediation: "Replace `shidou::init_tracing()` with \
                `shidou::init_tracing_to_stderr()` inside the `Mcp` \
                dispatch branch. Stdout is the JSON-RPC channel — any \
                log line on stdout breaks the protocol. See \
                theory/HEADLESS-INTROSPECTION.md §3."
                .into(),
        });
    }
}

// ─── 11. ScenarioCorpusPresent ────────────────────────────────────────
/// A GPU app crate with a `tests/scenarios/` directory must ship at
/// least one `*.scenario.yaml`. Empty corpus = no proof. The
/// scenario harness is a CI-gated substrate of provable behaviour;
/// without at least one scenario, the gate is trivially-passing.
pub struct ScenarioCorpusPresentChecker;

impl CseChecker for ScenarioCorpusPresentChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::ScenarioCorpusPresent
    }

    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>) {
        let dir = repo.path.join("tests/scenarios");
        if !dir.is_dir() {
            // Handled by the GpuAppHeadlessMode check — don't
            // duplicate the violation here.
            return;
        }
        let mut found = false;
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.ends_with(".scenario.yaml"))
                    .unwrap_or(false)
                {
                    found = true;
                    break;
                }
            }
        }
        if !found {
            violations.push(CseViolation::EmptyScenarioCorpus {
                repo: repo.name.clone(),
                remediation: "Add at least one *.scenario.yaml to \
                    tests/scenarios/. Capture a real reproducer with \
                    `<binary> record --output tests/scenarios/<bug>.scenario.yaml \
                    -- <repro-cmd>` and edit in expect: assertions. \
                    See pleme-io/mado/tests/scenarios/README.md."
                    .into(),
            });
        }
    }
}

// ─── 12. BuildSpecCanonicalUrl ───────────────────────────────────────
/// Asserts that every committed `Cargo.build-spec.json` emits
/// registry URLs in the canonical `static.crates.io` form. The old
/// `/api/v1/.../download` redirect endpoint is now rate-limited and
/// 403's against nixpkgs' UA-less fetchurl. gen-cargo 70774a2+ emits
/// the canonical form; this checker gates fleet drift back into the
/// rate-limited URL space.
pub struct BuildSpecCanonicalUrlChecker;

impl CseChecker for BuildSpecCanonicalUrlChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::BuildSpecCanonicalUrl
    }

    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>) {
        let spec = match &repo.cargo_build_spec_json {
            Some(s) => s,
            None => return,
        };
        // Cheap substring count — gen always emits the URL field on
        // one line per crate, so this is exact.
        let count = spec.matches("https://crates.io/api/v1/crates/").count();
        if count > 0 {
            violations.push(CseViolation::BuildSpecApiUrl {
                repo: repo.name.clone(),
                count,
                remediation:
                    "Run `gen build .` (gen >= 70774a2) to regenerate Cargo.build-spec.json with canonical static.crates.io URLs."
                        .into(),
            });
        }
    }
}

// ─── 13. BuildSpecSchemaVersion ──────────────────────────────────────
/// Asserts that every committed `Cargo.build-spec.json` carries the
/// current gen-cargo `SCHEMA_VERSION` (today: 3). Older specs lack
/// the typed `build_rust_crate_args` field and force substrate's
/// `legacyArgs` backward-compat path — duplicate computation that
/// the prime directive forbids past M6.
pub struct BuildSpecSchemaVersionChecker {
    expected: u32,
}

impl Default for BuildSpecSchemaVersionChecker {
    fn default() -> Self {
        // The expected SCHEMA_VERSION mirrors gen-cargo's constant.
        // Bump in lock-step when gen bumps.
        Self { expected: 3 }
    }
}

impl CseChecker for BuildSpecSchemaVersionChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::BuildSpecSchemaVersion
    }

    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>) {
        let spec = match &repo.cargo_build_spec_json {
            Some(s) => s,
            None => return,
        };
        // Lightweight version extraction — avoids pulling in serde_json
        // dep into cse-lint just for this check.
        let found: u32 = spec
            .lines()
            .find_map(|l| {
                let trimmed = l.trim_start();
                trimmed.strip_prefix("\"version\": ").and_then(|tail| {
                    tail.trim_end_matches(',').trim().parse::<u32>().ok()
                })
            })
            .unwrap_or(0);
        if found < self.expected {
            violations.push(CseViolation::BuildSpecStaleSchema {
                repo: repo.name.clone(),
                found,
                expected: self.expected,
                remediation: format!(
                    "Run `gen build .` to regenerate Cargo.build-spec.json against SCHEMA_VERSION {}.",
                    self.expected
                ),
            });
        }
    }
}

// ─── 14. NoLockfileBuilderDirectImport ───────────────────────────────
/// Asserts that consumer flakes don't import substrate's
/// `lockfile-builder.nix` directly. The canonical 4-line shape goes
/// through `substrate.rust.tool` / `substrate.rust.workspace` so that
/// substrate-side abstractions (override composition, schema-version
/// gating, URL canonicalization) apply uniformly.
pub struct NoLockfileBuilderDirectImportChecker;

fn lockfile_builder_direct_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"\$\{substrate\}/lib/build/rust/lockfile-builder\.nix"#)
            .expect("lockfile-builder direct-import regex must compile")
    })
}

impl CseChecker for NoLockfileBuilderDirectImportChecker {
    fn kind(&self) -> CseCheckKind {
        CseCheckKind::NoLockfileBuilderDirectImport
    }

    fn check(&self, repo: &RepoContext, violations: &mut Vec<CseViolation>) {
        let flake = match &repo.flake_nix {
            Some(f) => f,
            None => return,
        };
        if lockfile_builder_direct_re().is_match(flake) {
            violations.push(CseViolation::LockfileBuilderDirectImport {
                repo: repo.name.clone(),
                remediation:
                    "Replace `import \"${substrate}/lib/build/rust/lockfile-builder.nix\"` with the canonical `substrate.rust.tool { src = ./.; }` shape (or `substrate.rust.workspace`)."
                        .into(),
            });
        }
    }
}

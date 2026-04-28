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

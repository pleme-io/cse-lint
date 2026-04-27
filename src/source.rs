//! Repo enumeration: walk a workspace root and load each repo's
//! audit-relevant content into a [`RepoContext`].
//!
//! A "repo" here is any subdirectory of the given root that contains a
//! `.git/` entry. We don't recurse into nested git repos (worktrees,
//! submodules) — the audit treats each top-level repo as the unit of
//! state.

use crate::model::RepoContext;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Trait abstracting repo discovery + content loading.
///
/// The [`FilesystemSource`] impl walks `~/code/github/pleme-io/`. Tests
/// can provide a `MockSource` that returns fixed contexts.
pub trait RepoSource {
    fn repos(&self) -> Result<Vec<RepoContext>>;
}

/// Reads repos from a local clone tree (e.g. `~/code/github/pleme-io/`).
pub struct FilesystemSource {
    root: PathBuf,
    /// Optional explicit allowlist; if non-empty, only these repos
    /// are audited. Leaves room for `cse-lint audit --repo nami`.
    pub only: Vec<String>,
}

impl FilesystemSource {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            only: Vec::new(),
        }
    }

    pub fn with_only(mut self, only: Vec<String>) -> Self {
        self.only = only;
        self
    }
}

impl RepoSource for FilesystemSource {
    fn repos(&self) -> Result<Vec<RepoContext>> {
        let mut out = Vec::new();
        let entries = std::fs::read_dir(&self.root)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            // Skip dotted directories (.claude, .git, etc.)
            let name = match path.file_name().and_then(|s| s.to_str()) {
                Some(n) if !n.starts_with('.') => n.to_string(),
                _ => continue,
            };
            // Allowlist filter
            if !self.only.is_empty() && !self.only.contains(&name) {
                continue;
            }
            // Must be a git repo
            if !path.join(".git").exists() {
                continue;
            }
            out.push(load_context(&path, name)?);
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
}

fn load_context(path: &Path, name: String) -> Result<RepoContext> {
    let read_optional = |rel: &str| -> Option<String> {
        let p = path.join(rel);
        if p.exists() {
            std::fs::read_to_string(&p).ok()
        } else {
            None
        }
    };
    Ok(RepoContext {
        path: path.to_path_buf(),
        name,
        claude_md: read_optional("CLAUDE.md"),
        flake_nix: read_optional("flake.nix"),
        module_nix: read_optional("module/default.nix"),
    })
}

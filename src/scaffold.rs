//! Scaffold primitive: generate a baseline CLAUDE.md for repos that
//! lack one.
//!
//! Per the Compounding Directive: the substrate's own audit tool
//! shouldn't just *find* missing CLAUDE.md files, it should produce
//! a typed minimum-viable CLAUDE.md with the CSE pointer pre-wired.
//! Operators refine the stub afterward; the substrate guarantees the
//! pointer is always present.
//!
//! Stub shape:
//!   # <repo-name>
//!
//!   > **★★★ CSE / Knowable Construction.** [pointer block]
//!
//!   <description from README first paragraph if present, else stub>
//!
//!   <!-- cse-lint scaffold: replace this stub with a fuller
//!        repo-specific CLAUDE.md. The pointer above is mandatory and
//!        must remain. -->

use crate::fix::standard_pointer_block;
use crate::model::RepoContext;
use std::path::PathBuf;

/// A planned CLAUDE.md scaffold (file creation).
#[derive(Debug, Clone)]
pub struct ScaffoldPlan {
    pub repo_name: String,
    pub path: PathBuf,
    pub content: String,
}

/// Build a scaffold plan for a repo that's missing CLAUDE.md.
/// Returns None if CLAUDE.md already exists.
pub fn plan_scaffold(repo: &RepoContext) -> Option<ScaffoldPlan> {
    if repo.claude_md.is_some() {
        return None;
    }
    let description = extract_description(repo).unwrap_or_else(|| {
        format!(
            "<!-- cse-lint scaffold: replace this with a one-paragraph \
description of what {} does. The CSE pointer block above is \
mandatory and must remain. -->",
            repo.name
        )
    });
    let title = humanize_repo_name(&repo.name);
    let content = format!(
        "# {title}\n\n{pointer}\n{description}\n",
        title = title,
        pointer = standard_pointer_block(),
        description = description,
    );
    Some(ScaffoldPlan {
        repo_name: repo.name.clone(),
        path: repo.path.join("CLAUDE.md"),
        content,
    })
}

/// Apply a scaffold plan: write the file. Idempotent — caller
/// guarantees the file doesn't exist via `plan_scaffold` returning
/// Some.
pub fn apply_scaffold(plan: &ScaffoldPlan) -> std::io::Result<()> {
    std::fs::write(&plan.path, &plan.content)
}

/// Try to read a description from the repo's README.md (first
/// substantive paragraph). Falls back to the flake.nix `description = ...`
/// attribute. Returns None if neither is present.
fn extract_description(repo: &RepoContext) -> Option<String> {
    if let Some(text) = std::fs::read_to_string(repo.path.join("README.md")).ok() {
        if let Some(p) = first_paragraph(&text) {
            return Some(p);
        }
    }
    if let Some(flake) = &repo.flake_nix {
        if let Some(d) = flake_description(flake) {
            return Some(d);
        }
    }
    None
}

fn first_paragraph(text: &str) -> Option<String> {
    let mut buf = String::new();
    let mut started = false;
    for line in text.lines() {
        let t = line.trim();
        // Skip H1 / blank prelude
        if !started && (t.is_empty() || t.starts_with('#')) {
            continue;
        }
        if t.is_empty() {
            if !buf.is_empty() {
                return Some(buf.trim().to_string());
            }
            continue;
        }
        // Skip badges / image-only lines
        if t.starts_with("![") || t.starts_with("[!") {
            continue;
        }
        started = true;
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(t);
        // Cap at 500 chars to avoid pulling in massive intro paragraphs
        if buf.len() > 500 {
            break;
        }
    }
    if buf.is_empty() {
        None
    } else {
        Some(buf.trim().to_string())
    }
}

fn flake_description(flake: &str) -> Option<String> {
    // Look for `description = "..."` near the top of the flake.
    let re = regex::Regex::new(r#"description\s*=\s*"([^"]+)""#).ok()?;
    let caps = re.captures(flake)?;
    Some(caps.get(1)?.as_str().to_string())
}

/// Convert a repo name like "blackmatter-cli" or "akeyless_nix" into a
/// human-readable H1 title. Heuristic: split on - / _, title-case each
/// word.
fn humanize_repo_name(name: &str) -> String {
    name.split(|c: char| c == '-' || c == '_')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(c) => {
                    let mut s = String::new();
                    s.push(c.to_ascii_uppercase());
                    s.push_str(chars.as_str());
                    s
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

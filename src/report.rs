//! Output formatters for [`CseAuditReport`].
//!
//! Two formats:
//!   - human: a colored-ish summary suitable for terminals
//!   - json: the report serialized for downstream tools (CI, dashboards)

use crate::model::{CseAuditReport, CseCheckKind, RepoResult};

pub fn render_human(report: &CseAuditReport) -> String {
    let mut out = String::new();
    out.push_str("════════════════════════════════════════════════════════════\n");
    out.push_str(" cse-lint audit report — Constructive Substrate Engineering\n");
    out.push_str("════════════════════════════════════════════════════════════\n\n");
    out.push_str(&format!(
        "audited {} repos at {} ({} passing)\n\n",
        report.total_repos, report.run_at, report.passing_repos,
    ));

    // Summary table
    out.push_str("violations by check:\n");
    for kind in CseCheckKind::all() {
        let n = report.violations_by_kind.get(&kind).copied().unwrap_or(0);
        let marker = if n == 0 { "✓" } else { "✗" };
        out.push_str(&format!(
            "  {marker} {:<24} {} ({} invariant)\n",
            kind.label(),
            n,
            kind.invariant(),
        ));
    }
    out.push('\n');

    // Per-repo violations
    let failing: Vec<&RepoResult> = report.repos.iter().filter(|r| !r.pass).collect();
    if failing.is_empty() {
        out.push_str("all repos clean.\n");
    } else {
        out.push_str(&format!("failing repos ({}):\n\n", failing.len()));
        for repo in failing {
            out.push_str(&format!("  ▸ {}\n", repo.repo_name));
            for v in &repo.violations {
                let (msg, rem) = violation_message(v);
                out.push_str(&format!("      [{}] {}\n", v.kind().label(), msg));
                out.push_str(&format!("        → {}\n", rem));
            }
            out.push('\n');
        }
    }

    out
}

pub fn render_json(report: &CseAuditReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

fn violation_message(v: &crate::model::CseViolation) -> (String, String) {
    use crate::model::CseViolation::*;
    match v {
        MissingCsePointer { remediation, .. } => {
            ("CLAUDE.md is missing the CSE pointer".into(), remediation.clone())
        }
        HandRoll { pattern, remediation, .. } => (pattern.clone(), remediation.clone()),
        ManifestInconsistency { app, issue, remediation, .. } => {
            (format!("manifest issue for app `{}`: {}", app, issue), remediation.clone())
        }
        LegacyModulePattern { location, remediation, .. } => {
            (location.clone(), remediation.clone())
        }
        MissingDeployBundle { cluster, expected_path, remediation, .. } => {
            (
                format!("deploy.cluster=\"{}\" but no bundle at {}", cluster, expected_path),
                remediation.clone(),
            )
        }
        MissingCaixaManifest { remediation, .. } => {
            ("repo lacks a caixa.lisp at the root".into(), remediation.clone())
        }
    }
}

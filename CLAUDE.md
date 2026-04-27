# cse-lint — Constructive Substrate Engineering audit linter

> **★★★ CSE / Knowable Construction.** This repo is the *measurement
> primitive* of Constructive Substrate Engineering. CSE methodology
> canonical at
> [`pleme-io/theory/CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md`](https://github.com/pleme-io/theory/blob/main/CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md);
> Compounding Directive at the org-level pleme-io/CLAUDE.md ★★★ section.
> cse-lint exists so adherence to CSE is mechanically checkable rather
> than aspirational — without it the directive has no falsifiable ground.

## What it does

Walks every git repo under a workspace root and audits four CSE
invariants:

| Check | Invariant | What it asserts |
|-------|-----------|-----------------|
| `claude-md-pointer` | models stay current | CLAUDE.md links to CSE methodology + Compounding Directive |
| `hand-roll` | solve problems once | flake.nix consumes a substrate helper instead of `rustPlatform.buildRustPackage` / `flake-utils.lib.eachSystem` |
| `manifest-membership` | acquire and contextualize | apps in `pleme-io/nix/lib/ecosystem.nix` are class-assigned and referenced by ≥1 profile (stub; full check pending Nix-eval support) |
| `module-trio-adoption` | idiom-first | flake.nix passes `module = { ... }` instead of `// { homeManagerModules.default = import ./module ... }` |

Each violation comes with a typed remediation hint pointing to the
canonical reference (substrate helper, manifest entry, migration
commit).

## Usage

```bash
# Audit the local pleme-io workspace
cse-lint audit ~/code/github/pleme-io

# Just one repo
cse-lint audit ~/code/github/pleme-io --only nami

# Multiple repos
cse-lint audit ~/code/github/pleme-io --only nami,fumi,hibiki

# JSON output for CI
cse-lint audit ~/code/github/pleme-io --json

# Strict mode (exit 1 if any violations)
cse-lint audit ~/code/github/pleme-io --strict
```

## Build

```bash
nix build .#cse-lint            # via substrate's rust-tool-release-flake
cargo build --release            # local dev
```

## Architecture

- `src/model.rs` — typed report shape. Every claim cse-lint makes is
  grounded in a value here.
- `src/source.rs` — repo enumeration. `RepoSource` trait + filesystem
  impl. Future: git remote / sparse-checkout sources.
- `src/check.rs` — the four checkers. `CseChecker` trait; one impl
  per invariant. Each checker is small and focused.
- `src/report.rs` — output formatters (human, JSON).
- `src/main.rs` — CLI orchestrator.

The architecture is extensible: adding a fifth invariant is one impl
of `CseChecker` plus one variant in `CseCheckKind` and
`CseViolation`.

## Pending work

- `manifest-membership` is a stub. Full implementation requires
  parsing the manifest's Nix value (either via `nix eval --json`
  sub-process or by linking against a Nix evaluator). Once landed,
  the audit confirms every app in `ecosystem.nix` has a class +
  profile-membership chain.
- Renderer-reliability checks (substrate's renderer tests). Per the
  CSE doc's renderer-reliability section, every renderer needs five
  rigor levels of testing. cse-lint should audit *which* substrate
  helpers ship those tests and which don't.
- Trend tracking. JSON output is timestamped; future versions could
  emit deltas vs the previous run, so adherence trajectory is
  measurable across commits.

# Development guide

- Rust is pinned by `rust-toolchain.toml`; keep `Cargo.lock` committed.
- Keep argument declarations in `crates/relens-cli/src/cli.rs`, reusable/fallible domain logic in
  the appropriate library crate, command orchestration in `crates/relens-cli/src/commands/`, and
  process setup only in `crates/relens-cli/src/main.rs`.
- Library errors use `thiserror`; binary-level context uses `anyhow`. Do not call
  `process::exit` below `main`.
- Results go to stdout and diagnostics to stderr. Preserve human and JSON output modes.
- Before committing, run the format, Clippy, test, cargo-deny, and release build commands in README.
- Treat `docs/design.md` as the target architecture and `tasks.md` as the implementation ledger.
- Implement product behavior in `features/*.feature` scenario-sized increments. Each completed scenario
  must have cucumber step coverage plus unit or property tests for its reusable domain behavior.
- Preserve the workspace dependency direction `cli -> lift/engine/vcs/store -> domain`; the domain
  crate must not depend on adapter crates. Shared cucumber fixtures belong in `tests/e2e`.

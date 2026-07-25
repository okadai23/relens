# Development guide

- Rust is pinned by `rust-toolchain.toml`; keep `Cargo.lock` committed.
- Keep argument declarations in `src/cli.rs`, reusable/fallible domain logic in the library,
  command orchestration in `src/commands/`, and process setup only in `src/main.rs`.
- Library errors use `thiserror`; binary-level context uses `anyhow`. Do not call
  `process::exit` below `main`.
- Results go to stdout and diagnostics to stderr. Preserve human and JSON output modes.
- Before committing, run the format, Clippy, test, cargo-deny, and release build commands in README.
- Treat `docs/design.md` as the target architecture and `tasks.md` as the implementation ledger.
- Implement product behavior in `features/*.feature` scenario-sized increments. Each completed scenario
  must have cucumber step coverage plus unit or property tests for its reusable domain behavior.
- The current single crate is transitional. When workspace crates are introduced, preserve the dependency
  direction `cli -> lift/engine/vcs/store -> domain`; the domain crate must not depend on adapter crates.

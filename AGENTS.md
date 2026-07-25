# Development guide

- Rust is pinned by `rust-toolchain.toml`; keep `Cargo.lock` committed.
- Keep argument declarations in `src/cli.rs`, reusable/fallible domain logic in the library,
  command orchestration in `src/commands/`, and process setup only in `src/main.rs`.
- Library errors use `thiserror`; binary-level context uses `anyhow`. Do not call
  `process::exit` below `main`.
- Results go to stdout and diagnostics to stderr. Preserve human and JSON output modes.
- Before committing, run the format, Clippy, test, cargo-deny, and release build commands in README.

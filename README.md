# relens

長期運用しやすい Rust 製 CLI のベースプロジェクトです。

```console
cargo run -- init
cargo run -- run --output json
```

処理結果は stdout、診断とエラーは stderr に出力します。`--quiet`、`--verbose`、
`--color auto|always|never` を全サブコマンドで利用できます。

## 開発

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo deny check
cargo build --release --locked
```

リリースは `dist init` で生成・更新する GitHub Actions を利用し、`v1.2.3` 形式の
タグを起点に配布物を作成します。設定は `dist-workspace.toml` にあります。


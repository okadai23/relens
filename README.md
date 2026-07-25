# relens

`relens` は、プロジェクトテンプレートと派生プロジェクトの間を双方向に同期する
Rust 製 CLI です。テンプレートからの生成と更新だけでなく、派生プロジェクトで行った
修正を由来情報に基づいてテンプレートへ持ち上げ、再レンダリングによって検証します。

> [!NOTE]
> 現在は CLI の土台のみが実装済みです。目標アーキテクチャと振る舞いは
> [`docs/design.md`](docs/design.md)、実装順序は [`tasks.md`](tasks.md)、受け入れ条件は
> [`features/`](features/) を参照してください。

## 中核操作

- **render (`new`)**: テンプレートと回答からプロジェクトおよび SourceMap を生成する。
- **update**: 新旧テンプレートの pristine レンダリングと作業ツリーを 3-way マージする。
- **lift**: 作業ツリーの drift を TemplatePatch に逆写像し、PutGet 則で検証する。
- **matrix**: 代表的な回答の組み合わせをレンダリングし、未実体化の分岐も検査する。

正しさの基準となる lens 則は次のとおりです。

```text
PutGet: render(apply(template, lift(drift)), answers) == project_after_fix
GetPut: lift(diff(render(t, a), render(t, a))) == empty_patch
```

## 現在利用できる開発用コマンド

現在のスキャフォールドは次のコマンドを提供します。

```console
cargo run -- init
cargo run -- run --output json
```

処理結果は stdout、診断とエラーは stderr に出力します。`--quiet`、`--verbose`、
`--color auto|always|never` を全サブコマンドで利用できます。

## 開発

変更をコミットする前に、以下をすべて実行します。

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo deny check
cargo build --release --locked
```

今後の機能実装は `features/*.feature` のシナリオを小さな単位として進めます。シナリオを
実装するときは、対応する cucumber-rs のステップ、ドメイン層のユニットテスト、および
必要に応じて proptest のプロパティテストも同じ変更に含めます。

リリースは `dist init` で生成・更新する GitHub Actions を利用し、`v1.2.3` 形式の
タグを起点に配布物を作成します。設定は `dist-workspace.toml` にあります。

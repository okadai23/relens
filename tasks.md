# relens 実装タスク

このファイルを実装進捗の正とする。機能タスクは `features/*.feature` のシナリオ単位で完了させ、
完了時には cucumber E2E と対応する unit/property test の実行結果を追記する。

## 基盤

- [x] **[DONE] Cucumber world と単独 E2E 実行を安定化する**
  - `Default` でシナリオごとの一時 root を必ず初期化し、実シナリオでも fixture を利用可能にした。
  - E2E test target を CLI package に接続し、`cargo test -p relens-cli --test features` 単独でも binary を確実に解決できるようにした。

- [x] **[DONE] 設計と実行仕様をリポジトリへ記録する**
  - `docs/design.md` に境界、モデル、不変条件、逆写像方式を記録。
  - `features/` に日本語 Gherkin の受け入れ条件を配置。
  - 現在のスキャフォールドに対する Rust 品質ゲートを実行。
- [x] **[DONE] Cargo workspace と crate 境界を導入する**
  - `domain`, `engine`, `lift`, `vcs`, `store`, `cli` を依存方向に従って分割する。
  - 現在の stdout/stderr、人間向け/JSON 出力の契約を維持する。
  - `cargo test --workspace --all-features --locked` で crate unit test と CLI 契約を確認。
- [x] **[DONE] cucumber-rs E2E harness を導入する**
  - `RelensWorld`、実 git fixture builder、CLI runner、ファイル木 assertion を用意する。
  - 日本語キーワードの全 feature を discovery できる smoke test を追加する。
  - `tests/e2e/tests/features.rs` で cucumber-rs の World と Gherkin parser を検証。

## M1 — render、SourceMap、drift

- [x] **[DONE] `render.feature`: 回答を与えてプロジェクトを生成する**
- [x] **[DONE] `render.feature`: 生成結果は決定的である**
- [x] **[DONE] `render.feature`: 由来マップが出力全体を被覆する**
- [x] **[DONE] `roundtrip.feature`: GetPut 則 — ドリフトがなければパッチは空**
- [x] **[DONE] Questionnaire / AnswerSet / TemplateRef の型と検証を実装する**
- [x] **[DONE] Jinja サブセット parser と非対応構文の診断を実装する**
- [x] **[DONE] `.relens/answers.toml` と `lock.json` の永続化を実装する**
  - 型付き回答、決定的なファイル走査、計装 render、空ドリフトの GetPut を実装。
  - `relens-domain`、`relens-engine`、`relens-store` と CLI の `new` / `drift` / `lift` を更新。
  - crate unit test と `cargo test -p relens-cli --test features` の M1 シナリオで検証。
  - Windows でもテンプレートパスと lock のキーを `/` 区切りへ正規化し、生成直後の誤検出を防止。

## M2 — update

- [x] **[DONE] `update.feature`: ドリフトのないプロジェクトへの更新**
- [x] **[DONE] `update.feature`: 衝突しないローカル修正の自動マージ**
- [x] **[DONE] `update.feature`: 同一行の衝突と conflict marker の報告**
- [x] **[DONE] TemplateSource と git adapter を実装する**
- [x] **[DONE] pristine/base/project の 3-way merge を unit test する**
  - 記録済み commit とテンプレート HEAD を Git adapter で取得し、回答を再利用して双方を render する `relens update` を追加。
  - 片側変更と非重複行変更を自動統合し、競合時は marker を書き込んで衝突ファイルを stdout に報告。
  - engine unit test と `cargo test -p relens-cli --test features` の M2 受け入れシナリオで検証。

## M3 — Auto lift と verify

- [x] **[DONE] `lift.feature`: リテラル部分の修正を Auto で持ち上げる**
- [x] **[DONE] `lift.feature`: 変数由来値を逆置換する**
- [x] **[DONE] `lift.feature`: Jinja メタ文字を raw block で保護する**
- [x] **[DONE] `lift.feature`: 追加ファイルを Unmappable として扱う**
- [x] **[DONE] `roundtrip.feature`: PutGet のシナリオアウトライン**
- [x] **[DONE] PutGet/GetPut の proptest generator と shrinking を実装する**
  - SourceMap の式由来を使う逆置換、Jinja raw 保護、追加ファイルの提案付き分類を `relens-lift` に実装。
  - CLI が記録済みテンプレートから検証済み `.relens/template.patch` を生成し、分類と検証結果を報告するよう更新。
  - M3 の cucumber シナリオと PutGet/GetPut property test、workspace 品質ゲートで検証。
  - `relens lift` が生成ファイルやメタデータ、TemplatePatch のシンボリックリンクを拒否し、
    プロジェクト外を読み書きしないことを cucumber と unit test で検証。

## M4 — review、session、export

- [ ] **[TODO] `lift.feature`: 偶然一致を Ambiguous として候補提示する**
- [ ] **[TODO] `lift.feature`: KeepLiteral 裁定後に session を再開する**
- [ ] **[TODO] `lift.feature`: 検証失敗パッチの export を禁止する**
- [ ] **[TODO] `lift.feature`: Verified session を git branch へ export する**
- [ ] **[TODO] LiftSession の状態遷移と永続化を unit test する**

## M5 — matrix と拡張

- [ ] **[TODO] `matrix.feature`: 未実体化分岐の破損を検出する**
- [ ] **[TODO] `matrix.feature`: pairwise 回答集合を生成する**
- [ ] **[TODO] Migration の検出・実行・失敗時 rollback を実装する**
- [ ] **[TODO] 検証ゲートを迂回できない `LiftSuggester` port を定義する**

## 継続的な完了条件

各タスクを `[DONE]` にする前に次を満たす。

1. 対応シナリオが cucumber-rs で通る。
2. 再利用可能なドメイン動作に unit test、lens 則に property test がある。
3. human/JSON 出力と stdout/stderr の契約を維持する。
4. README に列挙された format、Clippy、test、cargo-deny、release build が通る。

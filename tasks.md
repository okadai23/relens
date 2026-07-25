# relens 実装タスク

このファイルを実装進捗の正とする。機能タスクは `features/*.feature` のシナリオ単位で完了させ、
完了時には cucumber E2E と対応する unit/property test の実行結果を追記する。

## 基盤

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

- [ ] **[TODO] `render.feature`: 回答を与えてプロジェクトを生成する**
- [ ] **[TODO] `render.feature`: 生成結果は決定的である**
- [ ] **[TODO] `render.feature`: 由来マップが出力全体を被覆する**
- [ ] **[TODO] `roundtrip.feature`: GetPut 則 — ドリフトがなければパッチは空**
- [ ] **[TODO] Questionnaire / AnswerSet / TemplateRef の型と検証を実装する**
- [ ] **[TODO] Jinja サブセット parser と非対応構文の診断を実装する**
- [ ] **[TODO] `.relens/answers.toml` と `lock.json` の永続化を実装する**

## M2 — update

- [ ] **[TODO] `update.feature`: ドリフトのないプロジェクトへの更新**
- [ ] **[TODO] `update.feature`: 衝突しないローカル修正の自動マージ**
- [ ] **[TODO] `update.feature`: 同一行の衝突と conflict marker の報告**
- [ ] **[TODO] TemplateSource と git adapter を実装する**
- [ ] **[TODO] pristine/base/project の 3-way merge を unit test する**

## M3 — Auto lift と verify

- [ ] **[TODO] `lift.feature`: リテラル部分の修正を Auto で持ち上げる**
- [ ] **[TODO] `lift.feature`: 変数由来値を逆置換する**
- [ ] **[TODO] `lift.feature`: Jinja メタ文字を raw block で保護する**
- [ ] **[TODO] `lift.feature`: 追加ファイルを Unmappable として扱う**
- [ ] **[TODO] `roundtrip.feature`: PutGet のシナリオアウトライン**
- [ ] **[TODO] PutGet/GetPut の proptest generator と shrinking を実装する**

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

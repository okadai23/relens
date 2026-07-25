# relens 設計

## 1. 目的とスコープ

`relens` は Template と派生 Project の双方向同期を行う。一般的なテンプレートツールの
render と下り方向の update に加え、Project の修正を TemplatePatch に変換する **lift**、
編集の由来追跡、ラウンドトリップ検証を中核機能とする。

| 機能 | cookiecutter | copier / cruft | relens |
|---|---:|---:|---:|
| 生成 | ✓ | ✓ | ✓ |
| Template → Project | — | ✓ | ✓ |
| Project → Template | — | — | ✓ |
| provenance | — | — | ✓ |
| ラウンドトリップ検証 | — | — | ✓ |

Jinja2 全体の互換性および AST レベルの意味論的リファクタリングは当面の対象外とする。

### lens 則

正しさは次の二つの性質で定義する。

```text
PutGet: render(apply(template, lift(drift)), answers) == project_after_fix
GetPut: lift(diff(render(t, a), render(t, a))) == empty_patch
```

PutGet は lift 元の単一 AnswerSet に対して機械検証する。他の回答での安全性は AnswerMatrix
で補完する。

## 2. アーキテクチャ

I/O とドメインロジックを分離するヘキサゴナルアーキテクチャを採用する。現在の単一 crate
は移行用のスキャフォールドであり、M1 の開始時に次の workspace へ段階的に分割する。

```text
crates/
├── relens-domain  # 値・集約・ポート・純粋ロジック
├── relens-engine  # Jinja サブセット、計装 render、SourceMap
├── relens-lift    # drift、逆写像、verify
├── relens-vcs     # gix による TemplateSource 等のアダプタ
├── relens-store   # AnswerSet、lock、LiftSession の永続化
└── relens-cli     # clap、対話レビュー、出力
tests/e2e/         # cucumber-rs の world と step 実装
features/          # 日本語 Gherkin の実行仕様
```

依存方向は `cli → lift/engine/vcs/store → domain` に限定し、domain は I/O アダプタに依存しない。

### ドメイン側ポート

```rust,ignore
pub trait TemplateSource {
    fn fetch(&self, reference: &TemplateRef) -> Result<TemplateTree, FetchError>;
    fn versions(&self, locator: &TemplateLocator)
        -> Result<Vec<TemplateVersion>, FetchError>;
}

pub trait Renderer {
    fn render(&self, template: &TemplateTree, answers: &AnswerSet)
        -> Result<RenderedInstance, RenderError>;
}

pub trait Workspace {
    fn read_tree(&self) -> Result<FileTree, IoError>;
    fn apply(&self, patch: &InstancePatch) -> Result<(), IoError>;
}
```

### CLI と操作

| CLI | ドメイン操作 |
|---|---|
| `relens new <template>` | Questionnaire → render → AnswerSet 永続化 |
| `relens check` | pristine 再構築と最新版の差分検査 |
| `relens update` | 3-way merge と Migration |
| `relens drift` | pristine と作業ツリーの Drift 抽出 |
| `relens lift` | Drift → TemplatePatch → verify |
| `relens verify <patch>` | PutGet の検証 |
| `relens matrix` | 回答集合の生成と一括 render |

## 3. ドメインモデル

### 用語と集約

| 用語 | 意味 |
|---|---|
| Template | Questionnaire と Jinja サブセットのファイル木。commit に固定された不変値 |
| AnswerSet | 回答と、その生成元 TemplateRef |
| Instance / Pristine | render 結果 / 記録済み入力から再構築した無垢な結果 |
| Project / Drift | ユーザーの作業ツリー / Pristine との差分 |
| SourceMap | 出力バイト範囲からテンプレート AST の Origin への対応 |
| TemplatePatch | テンプレートファイル木に対する編集集合 |
| LiftSession | drift 抽出からレビュー、検証、export までの再開可能な状態 |

集約境界は `Template`、`ProjectState`、`RenderedInstance`、`LiftSession` の四つとし、集約間は
ID で参照する。主要な値型は `TemplateRef`、`VarName`、`AnswerValue`、`TemplatePath`、
`NodeId`、`DriftHunk`、`TemplateEdit`、`Verification` とする。順序が永続化結果に影響する
コレクションには `BTreeMap` を用いる。

### Origin と lift の結果

SourceMap の Origin は次の三種類を区別する。

- `Literal`: テンプレートのソース範囲と 1:1 に対応する転写。
- `Expr`: 依存変数、フィルタ列、AST NodeId を持つ式評価結果。
- `Block`: if/for の条件、反復番号、NodeId を持つ制御ブロック由来。

各 hunk の結果は、一意に写像できる `Auto`、複数候補をレビューする `Ambiguous`、既定では
パッチに含めない `Unmappable` のいずれかになる。

### 不変条件

1. AnswerSet は常に TemplateRef を伴う。
2. SourceMap の出力範囲は昇順、非重複かつ隙間なくファイル全体を被覆する。
3. LiftSession は `Verified` を経なければ `Exported` へ遷移できない。
4. AnswerSet の具体値と一致する TemplateEdit 内の値は、変数へ逆置換済みか
   `KeepLiteral` として明示的に裁定済みでなければならない。

## 4. render と逆写像

### 計装 render

minijinja をラップし、パース時に AST NodeId を安定採番する。出力ストリームへの書き込み
単位で Literal、Expr、Block の Origin を記録することで、SourceMap を推測ではなく実測で
構築する。

式内部の値レイアウトが必要な場合は、各変数を私用領域文字で囲んだ衝突不能なセンチネル
へ置き換えて probe render する。組み込みフィルタはセンチネル本体を保存しつつ変換を追跡
できるものだけを許可する。

### hunk から TemplateEdit への変換

1. pristine の hunk 範囲を SourceMap の span 列へ分解する。
2. Literal のみなら対応するテンプレート範囲を書き換えて Auto とする。
3. Expr を含む場合、回答値と登録済みフィルタによる派生値を新テキストから逆置換する。
4. 値衝突、偶然一致、span/block 境界をまたぐ編集は Ambiguous とする。
5. 追加ファイルなど写像できない変更は、提案を添えた Unmappable とする。
6. 新しい Jinja メタ文字は raw block で保護する。

検証は `apply → render → Project とのバイト比較` という純粋な操作にし、不一致をファイル、
範囲、期待値、実際値からなる Divergence として返す。外部サジェスタを将来追加しても、
この検証を最終ゲートとする。

## 5. テンプレート言語

可逆性を優先し、以下の Jinja2 サブセットだけを受理する。

- `{{ var }}` と、純粋で単射性が既知のフィルタ列
- `{% if %}` / `{% elif %}` / `{% else %}` / `{% for %}`
- `{% raw %}`
- パス内の `{{ var }}`（条件付き生成は Questionnaire の `when` で表す）

任意の Python 式、macro、include、継承はパース時に拒否する。

## 6. 永続化

```text
project/.relens/
├── answers.toml # AnswerSet と TemplateRef。人間がレビュー可能
├── lock.json    # pristine ファイルの digest
└── sessions/    # 中断中の LiftSession

template/
├── relens.toml
├── migrations/
└── matrix.toml  # 省略時は pairwise 生成
```

## 7. AnswerMatrix

単一 AnswerSet の PutGet だけでは未実体化の分岐を保証できない。`relens matrix` は
Questionnaire から pairwise の代表回答を生成し、全 instance の render とテンプレート付属の
smoke test を実行する。lift 由来の変更は、この検査を通過してから統合する。

## 8. E2E 方針

実行仕様は [`../features`](../features/) の日本語 Gherkin を正とする。cucumber-rs の World は
シナリオごとの `TempDir`、実 git fixture、project directory、直近の CLI 結果、session ID を
保持する。CLI は assert_cmd でプロセスとして起動し、終了コード、stdout/stderr、ファイル木、
git 状態を境界の外側から検証する。

各シナリオは独立して実行可能にし、共通の fixture builder にだけセットアップを集約する。
roundtrip の代表例は E2E として、一般化した PutGet/GetPut は proptest として二層で検査する。

## 9. 実装マイルストーン

1. **M1**: domain、計装 engine、`new` / `drift`、GetPut
2. **M2**: `update` と 3-way merge
3. **M3**: Auto lift、`verify`、PutGet
4. **M4**: Ambiguous review、session 永続化、export
5. **M5**: matrix、migration、外部 LiftSuggester port

詳細な依存順序と完了条件は [`../tasks.md`](../tasks.md) で管理する。

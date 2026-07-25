# language: ja
機能: レンダリングと持ち上げのラウンドトリップ則
  ツールの開発者として
  render と lift が lens 則を満たすことを保証したい
  なぜならこれがツール全体の正しさの定義だからだ

  シナリオアウトライン: PutGet則 — 持ち上げて再レンダリングすると修正後と一致する
    前提 テンプレート "<template>" と回答 "<answers>" から生成されたプロジェクトがある
    かつ ユーザーが "<edit_kind>" 種の修正を加えた
    もし "relens lift" を実行し Auto の hunk のみでパッチを構成する
    かつ パッチ適用後のテンプレートを同じ回答で再レンダリングする
    ならば 再レンダリング結果は修正後のプロジェクトとバイト一致する

    例:
      | template   | answers                   | edit_kind         |
      | python-lib | project_name=myapp        | リテラル修正       |
      | python-lib | project_name=myapp        | 変数隣接行の修正   |
      | python-lib | project_name=app, pkg=app | 同値変数を含む修正 |

  シナリオ: GetPut則 — ドリフトがなければパッチは空
    前提 生成直後で未修正のプロジェクトがある
    もし "relens lift" を実行する
    ならば Drift は空である
    かつ TemplatePatch は生成されない

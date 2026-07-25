# language: ja
機能: テンプレートからのプロジェクト生成
  テンプレート利用者として
  テンプレートから新しいプロジェクトを生成したい
  なぜなら定型構成を素早く立ち上げたいからだ

  背景:
    前提 テンプレートリポジトリ "python-lib" が存在する:
      | ファイル                       | 内容                                      |
      | relens.toml                    | project_name: Str, use_docker: Bool       |
      | README.md.j2                   | # {{ project_name }}\n定型の説明文        |
      | {{ project_name }}/main.py.j2  | print("{{ project_name }}")              |
      | Dockerfile.j2                  | {% if use_docker %}FROM python{% endif %} |

  シナリオ: 回答を与えてプロジェクトを生成する
    もし "relens new python-lib" を回答 "project_name=myapp, use_docker=false" で実行する
    ならば 終了コードは 0 である
    かつ ファイル "README.md" の内容は "# myapp" で始まる
    かつ ディレクトリ "myapp" にファイル "main.py" が存在する
    かつ ファイル "Dockerfile" は存在しない
    かつ ".relens/answers.toml" にテンプレートのコミットIDが記録されている

  シナリオ: 生成結果は決定的である
    もし 同一のテンプレートと回答で 2 回生成する
    ならば 2 つの生成結果のファイル木はバイト単位で一致する

  シナリオ: 由来マップが出力全体を被覆する
    もし "relens new python-lib" を既定の回答で実行する
    ならば 生成された各テキストファイルの SourceMap は隙間なく全バイトを被覆している

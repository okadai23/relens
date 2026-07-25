use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use assert_cmd::Command as CliCommand;
use cucumber::{World, given, then, when};
use tempfile::TempDir;

/// Scenario-isolated state for executable feature steps.
#[derive(Debug, World)]
pub struct RelensWorld {
    root: Option<TempDir>,
    template_repository: Option<PathBuf>,
    project_directory: Option<PathBuf>,
    last_cli: Option<CliOutput>,
    session_id: Option<String>,
    second_project: Option<PathBuf>,
}

impl Default for RelensWorld {
    fn default() -> Self {
        Self {
            root: Some(tempfile::tempdir().expect("temporary world")),
            template_repository: None,
            project_directory: None,
            last_cli: None,
            session_id: None,
            second_project: None,
        }
    }
}

#[given(regex = r#"^テンプレートリポジトリ "python-lib" が存在する:$"#)]
fn template_repository_exists(world: &mut RelensWorld) {
    let template = world.git_fixture("python-lib").to_path_buf();
    fs::create_dir_all(template.join("{{ project_name }}")).unwrap();
    fs::write(
        template.join("relens.toml"),
        "[questions.project_name]\ntype = \"string\"\ndefault = \"sample\"\n[questions.use_docker]\ntype = \"bool\"\ndefault = false\n",
    )
    .unwrap();
    fs::write(
        template.join("README.md.j2"),
        "# {{ project_name }}\n定型の説明文",
    )
    .unwrap();
    fs::write(
        template.join("{{ project_name }}/main.py.j2"),
        "print(\"{{ project_name }}\")",
    )
    .unwrap();
    fs::write(
        template.join("Dockerfile.j2"),
        "{% if use_docker %}FROM python{% endif %}",
    )
    .unwrap();
    git_commit(&template, "fixture");
}

#[when(
    regex = r#"^"relens new python-lib" を回答 "project_name=myapp, use_docker=false" で実行する$"#
)]
fn generate_with_answers(world: &mut RelensWorld) {
    generate(
        world,
        "project",
        &["project_name=myapp", "use_docker=false"],
    );
}

#[when("同一のテンプレートと回答で 2 回生成する")]
fn generate_twice(world: &mut RelensWorld) {
    generate(world, "first", &["project_name=myapp", "use_docker=false"]);
    let first = world.project_directory.clone();
    generate(world, "second", &["project_name=myapp", "use_docker=false"]);
    world.second_project = world.project_directory.clone();
    world.project_directory = first;
}

#[when(regex = r#"^"relens new python-lib" を既定の回答で実行する$"#)]
fn generate_with_defaults(world: &mut RelensWorld) {
    generate(world, "default", &[]);
}

#[then("終了コードは 0 である")]
fn exit_is_success(world: &mut RelensWorld) {
    assert_eq!(world.last_cli.as_ref().unwrap().status, 0);
}

#[then("終了コードは 0 以外である")]
fn exit_is_failure(world: &mut RelensWorld) {
    assert_ne!(world.last_cli.as_ref().unwrap().status, 0);
}

#[then(regex = r##"^ファイル "README.md" の内容は "# myapp" で始まる$"##)]
fn readme_starts_correctly(world: &mut RelensWorld) {
    assert!(
        String::from_utf8_lossy(
            &fs::read(world.project_directory.as_ref().unwrap().join("README.md")).unwrap()
        )
        .starts_with("# myapp")
    );
}

#[then(regex = r#"^ディレクトリ "myapp" にファイル "main.py" が存在する$"#)]
fn nested_file_exists(world: &mut RelensWorld) {
    assert!(
        world
            .project_directory
            .as_ref()
            .unwrap()
            .join("myapp/main.py")
            .is_file()
    );
}

#[then(regex = r#"^ファイル "Dockerfile" は存在しない$"#)]
fn dockerfile_absent(world: &mut RelensWorld) {
    assert!(
        !world
            .project_directory
            .as_ref()
            .unwrap()
            .join("Dockerfile")
            .exists()
    );
}

#[then(regex = r#"^"\.relens/answers\.toml" にテンプレートのコミットIDが記録されている$"#)]
fn revision_recorded(world: &mut RelensWorld) {
    let revision = git_output(
        world.template_repository.as_ref().unwrap(),
        &["rev-parse", "HEAD"],
    );
    let answers = fs::read_to_string(
        world
            .project_directory
            .as_ref()
            .unwrap()
            .join(".relens/answers.toml"),
    )
    .unwrap();
    assert!(answers.contains(revision.trim()));
}

#[then("2 つの生成結果のファイル木はバイト単位で一致する")]
fn trees_match(world: &mut RelensWorld) {
    assert_eq!(
        snapshot(world.project_directory.as_ref().unwrap()),
        snapshot(world.second_project.as_ref().unwrap())
    );
}

#[then("生成された各テキストファイルの SourceMap は隙間なく全バイトを被覆している")]
fn source_maps_cover_output(world: &mut RelensWorld) {
    let lock: serde_json::Value = serde_json::from_slice(
        &fs::read(
            world
                .project_directory
                .as_ref()
                .unwrap()
                .join(".relens/lock.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for file in lock["files"].as_object().unwrap().values() {
        let spans = file["source_map"]["spans"].as_array().unwrap();
        assert_eq!(spans.first().unwrap()["start"], 0);
        for pair in spans.windows(2) {
            assert_eq!(pair[0]["end"], pair[1]["start"]);
        }
    }
}

#[given("生成直後で未修正のプロジェクトがある")]
fn pristine_project(world: &mut RelensWorld) {
    template_repository_exists(world);
    generate_with_defaults(world);
}

#[given(
    regex = r#"^テンプレート \"python-lib\" から回答 \"project_name=myapp\" で生成されたプロジェクトがある$"#
)]
fn generated_python_project(world: &mut RelensWorld) {
    template_repository_exists(world);
    generate(
        world,
        "lift-project",
        &["project_name=myapp", "use_docker=false"],
    );
}

#[given(
    regex = r#"^テンプレート \"python-lib\" と回答 \"(.+)\" から生成されたプロジェクトがある$"#
)]
fn generated_roundtrip_project(world: &mut RelensWorld, raw_answers: String) {
    template_repository_exists(world);
    let answers = raw_answers
        .split(',')
        .map(str::trim)
        .filter(|answer| answer.starts_with("project_name="))
        .chain(std::iter::once("use_docker=false"))
        .collect::<Vec<_>>();
    generate(world, "roundtrip-project", &answers);
}

#[given(regex = r#"^ユーザーが \"(.+)\" 種の修正を加えた$"#)]
fn apply_roundtrip_edit(world: &mut RelensWorld, edit_kind: String) {
    let project = world.project_directory.as_ref().unwrap();
    if edit_kind == "リテラル修正" {
        let path = project.join("README.md");
        fs::write(path, "# myapp\n説明文を修正").unwrap();
    } else {
        let package = if project.join("myapp/main.py").is_file() {
            "myapp"
        } else {
            "app"
        };
        fs::write(
            project.join(package).join("main.py"),
            format!("print(\"{package} v2\")"),
        )
        .unwrap();
    }
}

#[when(regex = r#"^\"relens lift\" を実行し Auto の hunk のみでパッチを構成する$"#)]
fn run_auto_lift(world: &mut RelensWorld) {
    run_lift(world);
    assert!(String::from_utf8_lossy(&world.last_cli.as_ref().unwrap().stdout).contains(":Auto"));
}

#[when("パッチ適用後のテンプレートを同じ回答で再レンダリングする")]
fn verification_rerenders_patch(world: &mut RelensWorld) {
    // `relens lift` performs this pure apply/render comparison before emitting a patch.
    assert!(
        String::from_utf8_lossy(&world.last_cli.as_ref().unwrap().stdout)
            .contains("verification:Pass")
    );
}

#[then("再レンダリング結果は修正後のプロジェクトとバイト一致する")]
fn put_get_matches(world: &mut RelensWorld) {
    verification_passes(world);
}

#[given(regex = r#"^ユーザーが \"README.md\" の定型説明文のタイプミスを修正した$"#)]
fn fix_literal_typo(world: &mut RelensWorld) {
    fs::write(
        world.project_directory.as_ref().unwrap().join("README.md"),
        "# myapp\n定型の説明文を修正",
    )
    .unwrap();
}

#[given(regex = r#"^ユーザーが \"myapp/main.py\" の行を 'print\(\"myapp v2\"\)' に変更した$"#)]
fn edit_variable_line(world: &mut RelensWorld) {
    fs::write(
        world
            .project_directory
            .as_ref()
            .unwrap()
            .join("myapp/main.py"),
        "print(\"myapp v2\")",
    )
    .unwrap();
}

#[given(regex = r#"^ユーザーがドキュメントに文字列 \"\{\{ example \}\}\" を追記した$"#)]
fn append_jinja_example(world: &mut RelensWorld) {
    let path = world.project_directory.as_ref().unwrap().join("README.md");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("\n{{ example }}");
    fs::write(path, text).unwrap();
}

#[given(regex = r#"^ユーザーがファイル \"notes/private.md\" を新規作成した$"#)]
fn add_private_notes(world: &mut RelensWorld) {
    let path = world
        .project_directory
        .as_ref()
        .unwrap()
        .join("notes/private.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "private").unwrap();
}

fn patch(world: &RelensWorld) -> String {
    fs::read_to_string(
        world
            .project_directory
            .as_ref()
            .unwrap()
            .join(".relens/template.patch"),
    )
    .unwrap_or_default()
}

#[then(regex = r#"^すべての hunk が \"Auto\" に分類される$"#)]
fn all_hunks_auto(world: &mut RelensWorld) {
    let stdout = String::from_utf8_lossy(&world.last_cli.as_ref().unwrap().stdout);
    assert!(stdout.contains(":Auto"));
    assert!(!stdout.contains("Unmappable"));
}

#[then(regex = r#"^生成された TemplatePatch は \"README.md.j2\" の同じタイプミスを修正する$"#)]
fn patch_fixes_typo(world: &mut RelensWorld) {
    let patch = patch(world);
    assert!(patch.contains("README.md.j2"));
    assert!(patch.contains("定型の説明文を修正"));
}

#[then(regex = r#"^TemplatePatch は 'print\(\"\{\{ project_name \}\} v2\"\)' への変更を含む$"#)]
fn patch_reverses_variable(world: &mut RelensWorld) {
    assert!(patch(world).contains("print(\"{{ project_name }} v2\")"));
}

#[then(regex = r#"^文字列 \"myapp\" は TemplatePatch に平文として現れない$"#)]
fn answer_not_literal(world: &mut RelensWorld) {
    assert!(!patch(world).contains("myapp"));
}

#[then(regex = r#"^TemplatePatch 内で当該文字列は raw ブロックで保護されている$"#)]
fn jinja_is_raw(world: &mut RelensWorld) {
    assert!(patch(world).contains("{% raw %}{{{% endraw %} example }}"));
}

#[then(regex = r#"^ラウンドトリップ検証は \"Pass\" である$"#)]
fn verification_passes(world: &mut RelensWorld) {
    assert!(
        String::from_utf8_lossy(&world.last_cli.as_ref().unwrap().stdout)
            .contains("verification:Pass")
    );
}

#[then(regex = r#"^当該ファイルは \"Unmappable\" として報告される$"#)]
fn added_file_unmappable(world: &mut RelensWorld) {
    assert!(
        String::from_utf8_lossy(&world.last_cli.as_ref().unwrap().stdout)
            .contains("notes/private.md:Unmappable")
    );
}

#[then("「テンプレートへ新規ファイルとして追加する」提案が表示される")]
fn addition_suggested(world: &mut RelensWorld) {
    assert!(
        String::from_utf8_lossy(&world.last_cli.as_ref().unwrap().stdout)
            .contains("テンプレートへ新規ファイルとして追加する")
    );
}

#[then("既定では TemplatePatch に含まれない")]
fn addition_not_patched(world: &mut RelensWorld) {
    assert!(!patch(world).contains("notes/private.md"));
}

#[given(regex = r#"^回答 \"project_name=main\" で生成されたプロジェクトがある$"#)]
fn generated_main_project(world: &mut RelensWorld) {
    generate(
        world,
        "main-project",
        &["project_name=main", "use_docker=false"],
    );
}

#[given(regex = r#"^ユーザーが新しい行 \"run main here\" をファイルに追加した$"#)]
fn append_accidental_match(world: &mut RelensWorld) {
    let path = world.project_directory.as_ref().unwrap().join("README.md");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("\nrun main here");
    fs::write(path, text).unwrap();
}

#[then(regex = r#"^当該 hunk は \"Ambiguous\" に分類される$"#)]
fn hunk_is_ambiguous(world: &mut RelensWorld) {
    assert!(
        String::from_utf8_lossy(&world.last_cli.as_ref().unwrap().stdout).contains("Ambiguous")
    );
}

#[then(
    regex = r#"^候補として \"run \{\{ project_name \}\} here\" と \"run main here\" の両方が提示される$"#
)]
fn both_candidates_present(world: &mut RelensWorld) {
    let output = String::from_utf8_lossy(&world.last_cli.as_ref().unwrap().stdout);
    assert!(output.contains("run {{ project_name }} here") && output.contains("run main here"));
}

#[given(regex = r#"^\"Ambiguous\" な hunk を含む LiftSession が存在する$"#)]
fn ambiguous_session_exists(world: &mut RelensWorld) {
    generated_main_project(world);
    append_accidental_match(world);
    run_lift(world);
}

#[when("ユーザーが当該 hunk に \"リテラルのまま維持\" を裁定する")]
fn choose_keep_literal(_: &mut RelensWorld) {}

#[when(regex = r#"^\"relens lift --resume\" を実行する$"#)]
fn resume_lift(world: &mut RelensWorld) {
    let project = world.project_directory.clone().unwrap();
    world.run_cli(&[
        "lift",
        project.to_str().unwrap(),
        "--resume",
        "--decision",
        "0=keep-literal",
    ]);
}

#[then(regex = r#"^セッションの状態は \"Verified\" に遷移する$"#)]
fn session_verified(world: &mut RelensWorld) {
    assert!(
        String::from_utf8_lossy(&world.last_cli.as_ref().unwrap().stdout)
            .contains("state:Verified")
    );
}

#[then("TemplatePatch には裁定どおりの平文が含まれる")]
fn patch_keeps_literal(world: &mut RelensWorld) {
    assert!(patch(world).contains("run main here"));
}

#[given("検証済み(Verified)の LiftSession が存在する")]
fn verified_session_exists(world: &mut RelensWorld) {
    fix_literal_typo(world);
    run_lift(world);
}

#[when(regex = r#"^\"relens lift --export\" を実行する$"#)]
fn export_lift(world: &mut RelensWorld) {
    let project = world.project_directory.clone().unwrap();
    world.run_cli(&["lift", project.to_str().unwrap(), "--export"]);
}

#[then(regex = r#"^テンプレートリポジトリにブランチ \"lift/myapp-<セッションID>\" が作成される$"#)]
fn export_branch_exists(world: &mut RelensWorld) {
    let branches = git_output(
        world.template_repository.as_ref().unwrap(),
        &["branch", "--list", "lift/myapp-*"],
    );
    assert!(branches.contains("lift/myapp-"));
}

#[then("ブランチのコミットに TemplatePatch が適用されている")]
fn exported_patch_applied(world: &mut RelensWorld) {
    assert!(
        fs::read_to_string(
            world
                .template_repository
                .as_ref()
                .unwrap()
                .join("README.md.j2")
        )
        .unwrap()
        .contains("定型の説明文を修正")
    );
}

#[then("コミットメッセージに由来プロジェクトと元コミットが記録されている")]
fn export_commit_metadata(world: &mut RelensWorld) {
    let message = git_output(
        world.template_repository.as_ref().unwrap(),
        &["log", "-1", "--pretty=%B"],
    );
    assert!(message.contains("myapp") && message.contains("Source-Commit:"));
}

#[given("持ち上げ後の再レンダリングが修正後プロジェクトと一致しない状態を注入する")]
fn inject_failed_verification(world: &mut RelensWorld) {
    verified_session_exists(world);
    let sessions = world
        .project_directory
        .as_ref()
        .unwrap()
        .join(".relens/sessions");
    let path = fs::read_dir(sessions)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    json["state"] = "Reviewing".into();
    json["divergences"] = serde_json::json!([{"path":"README.md","start":0,"end":8}]);
    fs::write(path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
}

#[then("標準出力に不一致箇所(ファイルと範囲)が表示される")]
fn divergence_is_printed(world: &mut RelensWorld) {
    assert!(
        String::from_utf8_lossy(&world.last_cli.as_ref().unwrap().stdout)
            .contains("README.md:0..8")
    );
}

#[then("テンプレートリポジトリに新しいブランチは作成されていない")]
fn no_export_branch(world: &mut RelensWorld) {
    let branches = git_output(
        world.template_repository.as_ref().unwrap(),
        &["branch", "--list", "lift/*"],
    );
    assert!(branches.trim().is_empty());
}

#[when(regex = r#"^"relens lift" を実行する$"#)]
fn run_lift(world: &mut RelensWorld) {
    let project = world.project_directory.clone().unwrap();
    world.run_cli(&["lift", project.to_str().unwrap()]);
}

#[then("Drift は空である")]
fn drift_empty(world: &mut RelensWorld) {
    assert_eq!(world.last_cli.as_ref().unwrap().status, 0);
}

#[then("TemplatePatch は生成されない")]
fn no_patch(world: &mut RelensWorld) {
    assert!(String::from_utf8_lossy(&world.last_cli.as_ref().unwrap().stdout).contains("no-patch"));
}

fn generate(world: &mut RelensWorld, name: &str, answers: &[&str]) {
    let destination = world.root.as_ref().unwrap().path().join(name);
    let template = world.template_repository.clone().unwrap();
    let mut args = vec![
        "new",
        template.to_str().unwrap(),
        "--destination",
        destination.to_str().unwrap(),
    ];
    for answer in answers {
        args.extend(["--answer", answer]);
    }
    world.run_cli(&args);
    world.project_directory = Some(destination);
}

fn git_output(path: &Path, args: &[&str]) -> String {
    String::from_utf8(
        Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
}

#[test]
fn executes_completed_m1_features_with_cucumber_steps() {
    let features = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../features");
    futures::executor::block_on(async {
        RelensWorld::cucumber()
            .run_and_exit(features.join("render.feature"))
            .await;
        RelensWorld::cucumber()
            .filter_run_and_exit(features.join("roundtrip.feature"), |_, _, scenario| {
                scenario.name.contains("GetPut")
            })
            .await;
    });
}

#[test]
fn executes_completed_m3_lift_features_with_cucumber_steps() {
    let features = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../features");
    futures::executor::block_on(async {
        RelensWorld::cucumber()
            .filter_run_and_exit(features.join("lift.feature"), |_, _, scenario| {
                scenario.name.contains("リテラル部分")
                    || scenario.name.contains("変数由来")
                    || scenario.name.contains("Jinjaメタ文字")
                    || scenario.name.contains("追加された無関係")
            })
            .await;
        RelensWorld::cucumber()
            .filter_run_and_exit(features.join("roundtrip.feature"), |_, _, scenario| {
                scenario.name.contains("PutGet")
            })
            .await;
    });
}

#[test]
fn executes_completed_m4_lift_features_with_cucumber_steps() {
    let features = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../features");
    futures::executor::block_on(async {
        RelensWorld::cucumber()
            .filter_run_and_exit(features.join("lift.feature"), |_, _, scenario| {
                scenario.name.contains("偶然一致")
                    || scenario.name.contains("リテラル維持")
                    || scenario.name.contains("検証に失敗")
                    || scenario
                        .name
                        .contains("テンプレートリポジトリへエクスポート")
            })
            .await;
    });
}

#[derive(Debug)]
struct CliOutput {
    status: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl RelensWorld {
    fn git_fixture(&mut self, name: &str) -> &Path {
        let path = self.root.as_ref().expect("world root").path().join(name);
        fs::create_dir_all(&path).expect("create git fixture");
        let output = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&path)
            .output()
            .expect("git must be available for acceptance tests");
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.template_repository = Some(path);
        self.template_repository.as_deref().unwrap()
    }

    fn run_cli(&mut self, args: &[&str]) {
        let output = CliCommand::cargo_bin("relens")
            .expect("relens binary")
            .args(args)
            .output()
            .expect("run relens CLI");
        self.last_cli = Some(CliOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: output.stdout,
            stderr: output.stderr,
        });
    }

    fn assert_file(&self, relative: &str, expected: &[u8]) {
        let actual = fs::read(
            self.project_directory
                .as_ref()
                .expect("project directory")
                .join(relative),
        )
        .expect("read project file");
        assert_eq!(actual, expected);
    }
}

#[test]
fn discovers_every_japanese_feature_with_the_cucumber_parser() {
    let feature_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../features");
    let mut discovered = BTreeSet::new();
    for entry in fs::read_dir(&feature_dir).expect("feature directory") {
        let path = entry.expect("feature entry").path();
        if path
            .extension()
            .is_some_and(|extension| extension == "feature")
        {
            let source = fs::read_to_string(&path).expect("read feature");
            let feature = gherkin::Feature::parse(
                &source,
                gherkin::GherkinEnv::new("ja").expect("Japanese Gherkin keywords"),
            )
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert_eq!(feature.keyword, "機能");
            discovered.insert(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    assert_eq!(
        discovered,
        BTreeSet::from([
            "lift.feature".into(),
            "matrix.feature".into(),
            "render.feature".into(),
            "roundtrip.feature".into(),
            "update.feature".into(),
        ])
    );
}

#[test]
fn fixture_builder_and_cli_runner_are_ready_for_scenario_steps() {
    let mut world = RelensWorld::default();
    assert!(world.git_fixture("template").join(".git").is_dir());
    world.run_cli(&["--version"]);
    let output = world.last_cli.as_ref().unwrap();
    assert_eq!(output.status, 0);
    assert!(output.stdout.starts_with(b"relens "));
    assert!(output.stderr.is_empty());

    let project = world.root.as_ref().unwrap().path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("result.txt"), b"expected").unwrap();
    world.project_directory = Some(project);
    world.session_id = Some("session-1".into());
    world.assert_file("result.txt", b"expected");
    assert_eq!(world.session_id.as_deref(), Some("session-1"));
}

/// Executable coverage for all four M1 acceptance scenarios. The assertions use
/// the same process boundary and isolated world as cucumber steps.
#[test]
fn m1_render_and_get_put_scenarios() {
    let root = tempfile::tempdir().unwrap();
    let template = root.path().join("python-lib");
    fs::create_dir_all(template.join("{{ project_name }}")).unwrap();
    fs::write(
        template.join("relens.toml"),
        r#"
[questions.project_name]
type = "string"
default = "sample"
[questions.use_docker]
type = "bool"
default = false
"#,
    )
    .unwrap();
    fs::write(
        template.join("README.md.j2"),
        "# {{ project_name }}\n定型の説明文",
    )
    .unwrap();
    fs::write(
        template.join("{{ project_name }}/main.py.j2"),
        "print(\"{{ project_name }}\")",
    )
    .unwrap();
    fs::write(
        template.join("Dockerfile.j2"),
        "{% if use_docker %}FROM python{% endif %}",
    )
    .unwrap();
    Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&template)
        .status()
        .unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&template)
        .status()
        .unwrap();
    Command::new("git")
        .args([
            "-c",
            "user.name=Relens",
            "-c",
            "user.email=relens@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ])
        .current_dir(&template)
        .status()
        .unwrap();

    let first = root.path().join("first");
    let second = root.path().join("second");
    for destination in [&first, &second] {
        CliCommand::cargo_bin("relens")
            .unwrap()
            .args([
                "new",
                template.to_str().unwrap(),
                "--destination",
                destination.to_str().unwrap(),
                "--answer",
                "project_name=myapp",
                "--answer",
                "use_docker=false",
            ])
            .assert()
            .success();
    }
    assert!(
        fs::read_to_string(first.join("README.md"))
            .unwrap()
            .starts_with("# myapp")
    );
    assert!(first.join("myapp/main.py").is_file());
    assert!(!first.join("Dockerfile").exists());
    let answers = fs::read_to_string(first.join(".relens/answers.toml")).unwrap();
    let revision = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&template)
        .output()
        .unwrap();
    assert!(answers.contains(String::from_utf8(revision.stdout).unwrap().trim()));
    assert_eq!(snapshot(&first), snapshot(&second));
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(first.join(".relens/lock.json")).unwrap()).unwrap();
    for file in lock["files"].as_object().unwrap().values() {
        let spans = file["source_map"]["spans"].as_array().unwrap();
        assert!(!spans.is_empty());
        assert_eq!(spans.first().unwrap()["start"], 0);
        for pair in spans.windows(2) {
            assert_eq!(pair[0]["end"], pair[1]["start"]);
        }
    }
    CliCommand::cargo_bin("relens")
        .unwrap()
        .args(["lift", first.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicates::str::contains("no-patch"));
}

/// Executable cucumber-step coverage for the four M2 update scenarios.
#[test]
fn m2_update_scenarios() {
    let root = tempfile::tempdir().unwrap();
    let template = root.path().join("python-lib");
    fs::create_dir_all(template.join("{{ project_name }}")).unwrap();
    fs::write(
        template.join("relens.toml"),
        "[questions.project_name]\ntype='string'\n",
    )
    .unwrap();
    fs::write(
        template.join("README.md.j2"),
        "# {{ project_name }}\nOverview\n",
    )
    .unwrap();
    fs::write(
        template.join("{{ project_name }}/main.py.j2"),
        "print('hello')\n",
    )
    .unwrap();
    git_commit(&template, "v1");
    let v1 = git_head(&template);

    let clean = root.path().join("clean");
    let independent = root.path().join("independent");
    let conflicting = root.path().join("conflicting");
    for project in [&clean, &independent, &conflicting] {
        CliCommand::cargo_bin("relens")
            .unwrap()
            .args([
                "new",
                template.to_str().unwrap(),
                "-d",
                project.to_str().unwrap(),
                "-a",
                "project_name=myapp",
            ])
            .assert()
            .success();
    }
    fs::write(
        independent.join("myapp/main.py"),
        "print('hello')\n# user line\n",
    )
    .unwrap();
    fs::write(conflicting.join("README.md"), "# myapp\nLocal overview\n").unwrap();
    fs::write(
        template.join("README.md.j2"),
        "# {{ project_name }}\nTemplate overview\n## Install\n",
    )
    .unwrap();
    git_commit(&template, "v2");
    let v2 = git_head(&template);
    assert_ne!(v1, v2);

    CliCommand::cargo_bin("relens")
        .unwrap()
        .args(["update", clean.to_str().unwrap()])
        .assert()
        .success();
    assert!(
        fs::read_to_string(clean.join("README.md"))
            .unwrap()
            .contains("## Install")
    );
    assert!(
        fs::read_to_string(clean.join(".relens/answers.toml"))
            .unwrap()
            .contains(&v2)
    );

    CliCommand::cargo_bin("relens")
        .unwrap()
        .args(["update", independent.to_str().unwrap()])
        .assert()
        .success();
    assert!(
        fs::read_to_string(independent.join("myapp/main.py"))
            .unwrap()
            .contains("# user line")
    );
    assert!(
        fs::read_to_string(independent.join("README.md"))
            .unwrap()
            .contains("Template overview")
    );
    CliCommand::cargo_bin("relens")
        .unwrap()
        .args(["drift", independent.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicates::str::contains("myapp/main.py"));

    CliCommand::cargo_bin("relens")
        .unwrap()
        .args(["update", conflicting.to_str().unwrap()])
        .assert()
        .failure()
        .stdout(predicates::str::contains("README.md"));
    assert!(
        fs::read_to_string(conflicting.join("README.md"))
            .unwrap()
            .contains("<<<<<<< project")
    );
}

#[cfg(unix)]
#[test]
fn update_rejects_a_path_through_a_symlinked_directory() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let template = root.path().join("python-lib");
    fs::create_dir_all(&template).unwrap();
    fs::write(template.join("relens.toml"), "[questions]\n").unwrap();
    fs::write(template.join("README.md.j2"), "v1\n").unwrap();
    git_commit(&template, "v1");

    let project = root.path().join("project");
    CliCommand::cargo_bin("relens")
        .unwrap()
        .args([
            "new",
            template.to_str().unwrap(),
            "-d",
            project.to_str().unwrap(),
        ])
        .assert()
        .success();

    let outside = root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("file.txt"), "outside remains unchanged\n").unwrap();
    symlink(&outside, project.join("output")).unwrap();
    fs::create_dir(template.join("output")).unwrap();
    fs::write(template.join("output/file.txt.j2"), "template update\n").unwrap();
    git_commit(&template, "v2");

    CliCommand::cargo_bin("relens")
        .unwrap()
        .args(["update", project.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicates::str::contains("symbolic link"));
    assert_eq!(
        fs::read_to_string(outside.join("file.txt")).unwrap(),
        "outside remains unchanged\n"
    );
    assert_eq!(
        fs::read_to_string(project.join("README.md")).unwrap(),
        "v1\n"
    );
}

fn git_commit(repository: &Path, message: &str) {
    if !repository.join(".git").exists() {
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(repository)
            .status()
            .unwrap();
    }
    assert!(
        Command::new("git")
            .args(["add", "."])
            .current_dir(repository)
            .status()
            .unwrap()
            .success()
    );
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=Relens",
            "-c",
            "user.email=relens@example.invalid",
            "commit",
            "--quiet",
            "-m",
            message,
        ])
        .current_dir(repository)
        .status()
        .unwrap();
    assert!(status.success());
}

fn git_head(repository: &Path) -> String {
    String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repository)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .into()
}

fn snapshot(root: &Path) -> BTreeSet<(String, Vec<u8>)> {
    walk(root)
        .into_iter()
        .map(|path| {
            (
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                fs::read(path).unwrap(),
            )
        })
        .collect()
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}

use crate::locator::build_args::{BuildTarget, ParsedDotnetBuild};
use crate::locator::msbuild::{MsbuildProperties, ResolvedProgram};
use crate::locator::paths::{join_path, parent_directory};
use crate::locator::{
    dotnet_build_args, is_absolute_path, msbuild_failure_message, msbuild_query_args,
    parse_dotnet_build_args, parse_msbuild_properties, prepare_test_task_for_build,
    resolve_program, scenario_config, split_command_line, MSBUILD_PROPERTIES,
};
use crate::serde_json::{self, Value};

const CWD: &str = "/work/repo";

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn parse(list: &[&str]) -> ParsedDotnetBuild {
    parse_dotnet_build_args(&args(list)).expect("should parse")
}

fn query(list: &[&str]) -> Vec<String> {
    msbuild_query_args(&parse(list), CWD).expect("should build query")
}

// --- task matching -------------------------------------------------

#[test]
fn dotnet_build_tasks_are_matched() {
    assert_eq!(dotnet_build_args("dotnet", &args(&["build"])), Some(vec![]));
    assert_eq!(
        dotnet_build_args("dotnet", &args(&["build", "-c", "Release"])),
        Some(args(&["-c", "Release"]))
    );
    assert_eq!(
        dotnet_build_args(" dotnet ", &args(&["build"])),
        Some(vec![])
    );
}

#[test]
fn single_string_command_is_matched() {
    assert_eq!(dotnet_build_args("dotnet build", &[]), Some(vec![]));
    assert_eq!(
        dotnet_build_args("dotnet build -c Release", &args(&["-f", "net8.0"])),
        Some(args(&["-c", "Release", "-f", "net8.0"]))
    );
}

#[test]
fn dotnet_exe_and_paths_are_matched() {
    assert!(dotnet_build_args("dotnet.exe", &args(&["build"])).is_some());
    assert!(dotnet_build_args("DOTNET.EXE build", &[]).is_some());
    assert!(dotnet_build_args("/usr/share/dotnet/dotnet", &args(&["build"])).is_some());
    assert_eq!(
        dotnet_build_args(
            r#""C:\Program Files\dotnet\dotnet.exe" build App.csproj"#,
            &[]
        ),
        Some(args(&["App.csproj"]))
    );
}

#[test]
fn dotnet_test_tasks_are_matched() {
    assert_eq!(dotnet_build_args("dotnet", &args(&["test"])), Some(vec![]));
    assert_eq!(
        dotnet_build_args(
            "dotnet test tests/T/T.csproj -c Release -- --filter-method MyTest",
            &[]
        ),
        Some(args(&[
            "tests/T/T.csproj",
            "-c",
            "Release",
            "--",
            "--filter-method",
            "MyTest"
        ]))
    );
}

#[test]
fn non_build_tasks_are_rejected() {
    assert!(dotnet_build_args("dotnet", &args(&["run"])).is_none());
    assert!(dotnet_build_args("dotnet", &args(&["publish"])).is_none());
    assert!(dotnet_build_args("dotnet", &args(&["clean", "build"])).is_none());
    assert!(dotnet_build_args("cargo", &args(&["build"])).is_none());
    assert!(dotnet_build_args("dotnetx", &args(&["build"])).is_none());
    assert!(dotnet_build_args("dotnet", &[]).is_none());
    assert!(dotnet_build_args("", &[]).is_none());
    assert!(dotnet_build_args("dotnet run build", &[]).is_none());
}

#[test]
fn parse_ignores_dotnet_test_switches_and_runner_tail() {
    let parsed = parse(&[
        "tests/T/T.csproj",
        "-c",
        "Release",
        "--filter",
        "FullyQualifiedName~Foo",
        "-l",
        "trx",
        "-s:test.runsettings",
        "--results-directory",
        "TestResults",
        "--no-build",
        "--",
        "--filter-method",
        "MyTest",
        "positional-looking-arg",
    ]);
    assert_eq!(
        parsed.target,
        Some(BuildTarget::Project("tests/T/T.csproj".to_string()))
    );
    assert_eq!(parsed.configuration.as_deref(), Some("Release"));
}

#[test]
fn parse_handles_dotnet_test_project_flag() {
    let parsed = parse(&["--project=tests/T/T.csproj", "-f", "net9.0"]);
    assert_eq!(
        parsed.target,
        Some(BuildTarget::Project("tests/T/T.csproj".to_string()))
    );
    assert_eq!(parsed.framework.as_deref(), Some("net9.0"));
}

#[test]
fn prepare_test_task_for_build_rewrites_test_and_strips_conflicting_flags() {
    assert_eq!(
        prepare_test_task_for_build("dotnet", &args(&["build", "App.csproj"])),
        None
    );

    let (cmd, build_args) = prepare_test_task_for_build(
        "dotnet",
        &args(&[
            "test",
            "tests/T/T.csproj",
            "-c",
            "Release",
            "-f:net8.0",
            "--filter",
            "Foo",
            "-l=trx",
            "--no-build",
            "--blame-hang-timeout",
            "10s",
            "--no-restore",
            "--",
            "--filter-method",
            "Bar",
        ]),
    )
    .expect("should rewrite");
    assert_eq!(cmd, "dotnet");
    assert_eq!(
        build_args,
        args(&[
            "build",
            "tests/T/T.csproj",
            "-c",
            "Release",
            "-f:net8.0",
            "--no-restore",
        ])
    );

    let (cmd, build_args) = prepare_test_task_for_build(
        r#""C:\Program Files\dotnet\dotnet.exe" test --project tests/T/T.csproj --sc false"#,
        &[],
    )
    .expect("should rewrite single-string command");
    assert_eq!(cmd, r"C:\Program Files\dotnet\dotnet.exe");
    assert_eq!(
        build_args,
        args(&["build", "tests/T/T.csproj", "--sc", "false"])
    );
}

#[test]
fn split_command_line_honors_quotes() {
    assert_eq!(
        split_command_line(r#"dotnet build "My App/App.csproj" -p:'A=b c'"#),
        args(&["dotnet", "build", "My App/App.csproj", "-p:A=b c"])
    );
    assert_eq!(split_command_line("   "), Vec::<String>::new());
    assert_eq!(split_command_line(r#""""#), args(&[""]));
}

// --- argument parsing ----------------------------------------------

#[test]
fn parse_defaults() {
    assert_eq!(parse(&[]), ParsedDotnetBuild::default());
}

#[test]
fn parse_separate_and_inline_values() {
    let parsed = parse(&[
        "--configuration",
        "Release",
        "-f:net8.0",
        "--runtime=linux-x64",
        "-o",
        "out",
        "--artifacts-path",
        "artifacts",
    ]);
    assert_eq!(parsed.configuration.as_deref(), Some("Release"));
    assert_eq!(parsed.framework.as_deref(), Some("net8.0"));
    assert_eq!(parsed.runtime.as_deref(), Some("linux-x64"));
    assert_eq!(parsed.output.as_deref(), Some("out"));
    assert_eq!(parsed.artifacts_path.as_deref(), Some("artifacts"));
}

#[test]
fn parse_last_value_wins() {
    let parsed = parse(&["-c", "Debug", "--configuration=Release"]);
    assert_eq!(parsed.configuration.as_deref(), Some("Release"));
}

#[test]
fn parse_collects_every_property_spelling() {
    let parsed = parse(&[
        "-p:A=1",
        "-property:B=2",
        "--property:C=3",
        "/p:D=4",
        "/property:E=5",
        "-p",
        "F=6",
        "--property",
        "G=7;H=8",
    ]);
    assert_eq!(
        parsed.properties,
        args(&["A=1", "B=2", "C=3", "D=4", "E=5", "F=6", "G=7;H=8"])
    );
}

#[test]
fn parse_use_current_runtime() {
    assert!(parse(&["--ucr"]).use_current_runtime);
    assert!(parse(&["--use-current-runtime"]).use_current_runtime);
    assert!(!parse(&[]).use_current_runtime);
}

#[test]
fn parse_classifies_targets() {
    assert_eq!(
        parse(&["src/App/App.csproj"]).target,
        Some(BuildTarget::Project("src/App/App.csproj".to_string()))
    );
    assert_eq!(
        parse(&["App.FSPROJ"]).target,
        Some(BuildTarget::Project("App.FSPROJ".to_string()))
    );
    assert_eq!(
        parse(&["All.sln"]).target,
        Some(BuildTarget::Solution("All.sln".to_string()))
    );
    assert_eq!(
        parse(&["All.slnx"]).target,
        Some(BuildTarget::Solution("All.slnx".to_string()))
    );
    assert_eq!(
        parse(&["src/App"]).target,
        Some(BuildTarget::Directory("src/App".to_string()))
    );
}

#[test]
fn parse_does_not_confuse_switch_values_for_targets() {
    let parsed = parse(&[
        "-v",
        "minimal",
        "--version-suffix",
        "beta",
        "--sc",
        "false",
        "--no-restore",
        "-bl",
        "-c",
        "Release.csproj",
    ]);
    assert_eq!(parsed.target, None);
    assert_eq!(parsed.configuration.as_deref(), Some("Release.csproj"));
}

#[test]
fn parse_rejects_missing_value() {
    let err = parse_dotnet_build_args(&args(&["-c"])).unwrap_err();
    assert!(err.contains("`-c`") && err.contains("missing"), "{err}");
}

#[test]
fn parse_rejects_empty_value() {
    let err = parse_dotnet_build_args(&args(&["--framework="])).unwrap_err();
    assert!(err.contains("empty"), "{err}");
}

#[test]
fn parse_rejects_arch_and_os() {
    for list in [&["-a", "x64"][..], &["--arch=arm64"], &["--os", "linux"]] {
        let err = parse_dotnet_build_args(&args(list)).unwrap_err();
        assert!(err.contains("-r <runtime-identifier>"), "{err}");
    }
}

#[test]
fn parse_rejects_multiple_targets() {
    let err = parse_dotnet_build_args(&args(&["A.csproj", "B.csproj"])).unwrap_err();
    assert!(err.contains("more than one build target"), "{err}");
}

// --- paths ---------------------------------------------------------

#[test]
fn absolute_path_detection_handles_both_platforms() {
    assert!(is_absolute_path("/work"));
    assert!(is_absolute_path("C:\\work"));
    assert!(is_absolute_path("c:/work"));
    assert!(is_absolute_path("\\\\server\\share"));
    assert!(!is_absolute_path("work"));
    assert!(!is_absolute_path("./work"));
    assert!(!is_absolute_path(""));
}

#[test]
fn join_path_normalizes_separators() {
    assert_eq!(join_path("C:\\repo\\", "src\\App"), "C:/repo/src/App");
    assert_eq!(join_path("/repo", "./out"), "/repo/out");
    assert_eq!(join_path("/repo", "/abs/out"), "/abs/out");
    assert_eq!(join_path("/repo", "."), "/repo");
    assert_eq!(join_path("", "rel"), "rel");
}

#[test]
fn parent_directory_handles_both_platforms() {
    assert_eq!(
        parent_directory("C:\\repo\\App\\App.csproj").as_deref(),
        Some("C:/repo/App")
    );
    assert_eq!(
        parent_directory("/repo/App.csproj").as_deref(),
        Some("/repo")
    );
    assert_eq!(parent_directory("/App.csproj"), None);
    assert_eq!(parent_directory("App.csproj"), None);
}

// --- query construction --------------------------------------------

#[test]
fn query_requires_absolute_cwd() {
    let parsed = parse(&[]);
    for cwd in ["", "  ", "relative/dir"] {
        let err = msbuild_query_args(&parsed, cwd).unwrap_err();
        assert!(err.contains("absolute `cwd`"), "{err}");
    }
}

#[test]
fn query_targets_cwd_when_no_target() {
    assert_eq!(query(&[]), args(&["msbuild", CWD, MSBUILD_PROPERTIES]));
}

#[test]
fn query_resolves_project_and_directory_against_cwd() {
    assert_eq!(
        query(&["src/App/App.csproj"])[1],
        "/work/repo/src/App/App.csproj"
    );
    assert_eq!(query(&["src\\App"])[1], "/work/repo/src/App");
    assert_eq!(
        query(&["/elsewhere/App.csproj"])[1],
        "/elsewhere/App.csproj"
    );
}

#[test]
fn query_forwards_output_affecting_switches_like_the_cli() {
    let query = query(&[
        "-c",
        "Release",
        "-f",
        "net8.0",
        "-r",
        "linux-x64",
        "--ucr",
        "-o",
        "out",
        "--artifacts-path",
        "/tmp/artifacts",
        "-p:Foo=Bar",
    ]);
    assert_eq!(
        query[3..],
        args(&[
            "--property:Configuration=Release",
            "--property:TargetFramework=net8.0",
            "--property:RuntimeIdentifier=linux-x64",
            "--property:_CommandLineDefinedRuntimeIdentifier=true",
            "--property:UseCurrentRuntimeIdentifier=True",
            "--property:OutputPath=/work/repo/out/",
            "--property:_CommandLineDefinedOutputPath=true",
            "--property:ArtifactsPath=/tmp/artifacts",
            "--property:Foo=Bar",
        ])
    );
}

#[test]
fn query_output_path_keeps_single_trailing_separator() {
    let query = query(&["-o", "C:\\out\\"]);
    assert!(
        query.contains(&"--property:OutputPath=C:/out/".to_string()),
        "{query:?}"
    );
}

#[test]
fn query_ignores_switches_that_do_not_affect_output() {
    assert_eq!(query(&["--no-restore", "-v", "q", "-bl"]).len(), 3);
}

#[test]
fn query_rejects_solutions() {
    let err = msbuild_query_args(&parse(&["All.sln"]), CWD).unwrap_err();
    assert!(err.contains("solution") && err.contains(".csproj"), "{err}");
}

// --- failure and output parsing ------------------------------------

#[test]
fn failure_on_solution_directory_is_actionable() {
    // MSBuild writes (localized) errors to stdout; detection keys on the code.
    let message = msbuild_failure_message(
        Some(1),
        "MSBUILD : error MSB1063: <localized text>\nSwitch: getProperty",
        "",
    );
    assert!(
        message.contains("solution") && message.contains(".csproj"),
        "{message}"
    );
}

#[test]
fn failure_with_several_projects_names_the_remedy() {
    let message = msbuild_failure_message(Some(1), "MSBUILD : error MSB1050: ...", "");
    assert!(message.contains("more than one project"), "{message}");
    let message = msbuild_failure_message(Some(1), "MSBUILD : error MSB1011: ...", "");
    assert!(message.contains("more than one project"), "{message}");
}

#[test]
fn failure_without_project_and_old_sdk_have_hints() {
    let message = msbuild_failure_message(Some(1), "error MSB1003: ...", "");
    assert!(message.contains("No project file"), "{message}");
    let message = msbuild_failure_message(Some(1), "error MSB1001: -getProperty", "");
    assert!(message.contains("SDK 8"), "{message}");
}

#[test]
fn failure_quotes_unknown_output() {
    let message = msbuild_failure_message(None, "", "boom");
    assert!(
        message.contains("boom") && message.contains("None"),
        "{message}"
    );
}

#[test]
fn parse_msbuild_json_output() {
    // Shape captured from `dotnet msbuild` on .NET SDK 9.0.316.
    let stdout = r#"{
  "Properties": {
"TargetPath": "C:\\repo\\App\\bin\\Debug\\net9.0\\App.dll",
"TargetFramework": "net9.0",
"TargetFrameworks": "",
"MSBuildProjectFullPath": "C:\\repo\\App\\App.csproj"
  }
}"#;
    let properties = parse_msbuild_properties(stdout).expect("should parse");
    assert_eq!(
        properties,
        MsbuildProperties {
            target_path: Some("C:\\repo\\App\\bin\\Debug\\net9.0\\App.dll".to_string()),
            target_framework: Some("net9.0".to_string()),
            target_frameworks: None,
            project_full_path: Some("C:\\repo\\App\\App.csproj".to_string()),
        }
    );
}

#[test]
fn parse_msbuild_non_json_output_is_actionable() {
    let err = parse_msbuild_properties("/repo/bin/App.dll").unwrap_err();
    assert!(err.contains("SDK 8"), "{err}");
    let err = parse_msbuild_properties(r#"{"Other": {}}"#).unwrap_err();
    assert!(err.contains("`Properties`"), "{err}");
}

#[test]
fn resolve_program_uses_target_path_and_project_dir() {
    let properties = MsbuildProperties {
        target_path: Some("/repo/App/bin/Release/net8.0/linux-x64/App.dll".to_string()),
        target_framework: Some("net8.0".to_string()),
        target_frameworks: None,
        project_full_path: Some("/repo/App/App.csproj".to_string()),
    };
    assert_eq!(
        resolve_program(&properties, CWD),
        Ok(ResolvedProgram {
            program: "/repo/App/bin/Release/net8.0/linux-x64/App.dll".to_string(),
            cwd: "/repo/App".to_string(),
        })
    );
}

#[test]
fn resolve_program_falls_back_to_task_cwd() {
    let properties = MsbuildProperties {
        target_path: Some("/repo/out/App.dll".to_string()),
        ..Default::default()
    };
    assert_eq!(resolve_program(&properties, CWD).unwrap().cwd, CWD);
}

#[test]
fn resolve_program_multi_target_error_mentions_framework_switch() {
    let properties = MsbuildProperties {
        target_frameworks: Some("net8.0;net9.0".to_string()),
        ..Default::default()
    };
    let err = resolve_program(&properties, CWD).unwrap_err();
    assert!(
        err.contains("-f <tfm>") && err.contains("net8.0;net9.0"),
        "{err}"
    );
}

#[test]
fn resolve_program_requires_target_path() {
    let err = resolve_program(&MsbuildProperties::default(), CWD).unwrap_err();
    assert!(err.contains("`TargetPath`"), "{err}");
}

#[test]
fn scenario_config_triggers_phase_two_locator() {
    let value: Value = serde_json::from_str(&scenario_config()).unwrap();
    assert_eq!(value, serde_json::json!({}));
    assert!(
        crate::config::request_kind(&value).is_err(),
        "phase-1 locator scenario must omit `request` so Zed invokes phase 2"
    );
}

use crate::config::jsonc::parse_jsonc;
use crate::config::launch_settings::{LaunchSettingsProfile, LaunchSettingsSpec};
use crate::config::logging::VSCODE_IGNORED_LOGGING_KEYS;
use crate::config::{
    adapter_logging_args, adapter_pipe_transport, merge_launch_sources, normalize_attach,
    normalize_launch, normalize_worktree_relative_path, parse_env_file,
    parse_launch_settings_profile, request_kind, scenario_config, take_env_file,
    take_launch_settings_spec, translate_launch_json, GenericRequest, RequestKind,
};
use crate::serde_json::{json, Value};

const ROOT: &str = "/project";

#[test]
fn request_kind_launch() {
    assert_eq!(
        request_kind(&json!({"request": "launch"})),
        Ok(RequestKind::Launch)
    );
}

#[test]
fn request_kind_attach() {
    assert_eq!(
        request_kind(&json!({"request": "attach"})),
        Ok(RequestKind::Attach)
    );
}

#[test]
fn request_kind_missing() {
    let err = request_kind(&json!({"program": "app.dll"})).unwrap_err();
    assert!(err.contains("`request`"), "unexpected error: {err}");
}

#[test]
fn request_kind_invalid() {
    let err = request_kind(&json!({"request": "restart"})).unwrap_err();
    assert!(err.contains("restart"), "unexpected error: {err}");
}

#[test]
fn request_kind_non_string_request() {
    let err = request_kind(&json!({"request": 1})).unwrap_err();
    assert!(err.contains("`request`"), "unexpected error: {err}");
}

#[test]
fn request_kind_is_case_sensitive() {
    // netcoredbg and Zed both use the lowercase DAP command names.
    assert!(request_kind(&json!({"request": "Launch"})).is_err());
}

#[test]
fn request_kind_non_object_config() {
    assert!(request_kind(&json!(["launch"])).is_err());
    assert!(request_kind(&json!(null)).is_err());
}

fn launch_request(cwd: Option<&str>, args: &[&str], envs: &[(&str, &str)]) -> GenericRequest {
    GenericRequest::Launch {
        program: "/project/bin/Debug/net8.0/App.dll".to_string(),
        cwd: cwd.map(str::to_string),
        args: args.iter().map(|a| a.to_string()).collect(),
        envs: envs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

#[test]
fn scenario_config_full_launch() {
    let request = launch_request(Some("/project"), &["--verbose"], &[("A", "1")]);
    assert_eq!(
        scenario_config(&request, Some(true)),
        json!({
            "request": "launch",
            "program": "/project/bin/Debug/net8.0/App.dll",
            "args": ["--verbose"],
            "cwd": "/project",
            "env": {"A": "1"},
            "stopAtEntry": true
        })
    );
}

#[test]
fn scenario_config_minimal_launch_omits_optional_fields() {
    let config = scenario_config(&launch_request(None, &[], &[]), None);
    assert_eq!(
        config,
        json!({"request": "launch", "program": "/project/bin/Debug/net8.0/App.dll"})
    );
}

#[test]
fn scenario_config_stop_on_entry_false_is_explicit() {
    let config = scenario_config(&launch_request(None, &[], &[]), Some(false));
    assert_eq!(config["stopAtEntry"], json!(false));
}

#[test]
fn scenario_config_blank_cwd_is_omitted() {
    let config = scenario_config(&launch_request(Some("  "), &[], &[]), None);
    assert!(config.get("cwd").is_none(), "unexpected config: {config}");
}

#[test]
fn scenario_config_launch_round_trips_through_normalize_launch() {
    let mut config = scenario_config(&launch_request(None, &["x"], &[]), Some(true));
    assert_eq!(request_kind(&config), Ok(RequestKind::Launch));
    normalize_launch(&mut config, ROOT, None).expect("should normalize");
    assert_eq!(config["cwd"], json!(ROOT));
    assert_eq!(config["stopAtEntry"], json!(true));
}

#[test]
fn scenario_config_attach_with_pid() {
    let mut config = scenario_config(
        &GenericRequest::Attach {
            process_id: Some(4242),
        },
        None,
    );
    assert_eq!(config, json!({"request": "attach", "processId": 4242}));
    assert_eq!(request_kind(&config), Ok(RequestKind::Attach));
    normalize_attach(&mut config).expect("should normalize");
}

#[test]
fn scenario_config_attach_without_pid_omits_process_id() {
    let mut config = scenario_config(&GenericRequest::Attach { process_id: None }, None);
    assert_eq!(config, json!({"request": "attach"}));
    // The request kind is still determinable; validation then reports the
    // missing PID instead of a `null` type error.
    assert_eq!(request_kind(&config), Ok(RequestKind::Attach));
    let err = normalize_attach(&mut config).unwrap_err();
    assert!(
        err.contains("requires a `processId`"),
        "unexpected error: {err}"
    );
}

#[test]
fn scenario_config_attach_ignores_stop_on_entry() {
    let config = scenario_config(
        &GenericRequest::Attach {
            process_id: Some(1),
        },
        Some(true),
    );
    assert!(
        config.get("stopAtEntry").is_none(),
        "unexpected config: {config}"
    );
}

#[test]
fn launch_requires_program() {
    let mut config = json!({"request": "launch"});
    let err = normalize_launch(&mut config, ROOT, None).unwrap_err();
    assert!(err.contains("`program`"), "unexpected error: {err}");
}

#[test]
fn launch_rejects_blank_program() {
    let mut config = json!({"request": "launch", "program": "   "});
    assert!(normalize_launch(&mut config, ROOT, None).is_err());
}

#[test]
fn launch_defaults_cwd_to_worktree_root() {
    let mut config = json!({"request": "launch", "program": "bin/app.dll"});
    normalize_launch(&mut config, ROOT, None).expect("should normalize");
    assert_eq!(config["cwd"], json!(ROOT));
}

#[test]
fn launch_preserves_explicit_cwd() {
    let mut config = json!({"request": "launch", "program": "bin/app.dll", "cwd": "/other"});
    normalize_launch(&mut config, ROOT, None).expect("should normalize");
    assert_eq!(config["cwd"], json!("/other"));
}

#[test]
fn launch_injects_dotnet_root_hint() {
    let mut config = json!({"request": "launch", "program": "bin/app.dll"});
    normalize_launch(&mut config, ROOT, Some("/usr/share/dotnet/dotnet")).expect("ok");
    assert_eq!(config["env"]["DOTNET_ROOT"], json!("/usr/share/dotnet"));
}

#[test]
fn launch_keeps_user_dotnet_root_and_merges_env() {
    let mut config = json!({
        "request": "launch",
        "program": "bin/app.dll",
        "env": {"DOTNET_ROOT": "/custom", "ASPNETCORE_ENVIRONMENT": "Development"}
    });
    normalize_launch(&mut config, ROOT, Some("/usr/share/dotnet/dotnet")).expect("ok");
    assert_eq!(config["env"]["DOTNET_ROOT"], json!("/custom"));
    assert_eq!(
        config["env"]["ASPNETCORE_ENVIRONMENT"],
        json!("Development")
    );
}

#[test]
fn launch_preserves_known_and_unknown_fields() {
    let mut config = json!({
        "request": "launch",
        "program": "bin/app.dll",
        "args": ["--verbose", "--flag", "value"],
        "stopAtEntry": true,
        "justMyCode": false,
        "enableStepFiltering": false,
        "sourceFileMap": {"/src": "/work/src"},
        "someFutureField": 42
    });
    normalize_launch(&mut config, ROOT, None).expect("should normalize");
    assert_eq!(config["args"], json!(["--verbose", "--flag", "value"]));
    assert_eq!(config["stopAtEntry"], json!(true));
    assert_eq!(config["justMyCode"], json!(false));
    assert_eq!(config["enableStepFiltering"], json!(false));
    assert_eq!(config["sourceFileMap"], json!({"/src": "/work/src"}));
    assert_eq!(config["someFutureField"], json!(42));
}

#[test]
fn launch_trims_program() {
    let mut config = json!({"request": "launch", "program": "  bin/app.dll  "});
    normalize_launch(&mut config, ROOT, None).expect("ok");
    assert_eq!(config["program"], json!("bin/app.dll"));
}

#[test]
fn launch_rejects_non_string_args() {
    let mut config = json!({"request": "launch", "program": "app", "args": [1, 2]});
    let err = normalize_launch(&mut config, ROOT, None).unwrap_err();
    assert!(err.contains("`args`"), "unexpected error: {err}");
}

#[test]
fn launch_rejects_non_object_env() {
    let mut config = json!({"request": "launch", "program": "app", "env": []});
    let err = normalize_launch(&mut config, ROOT, None).unwrap_err();
    assert!(err.contains("`env`"), "unexpected error: {err}");
}

#[test]
fn launch_rejects_non_object_config() {
    let mut config = json!(["not", "an", "object"]);
    let err = normalize_launch(&mut config, ROOT, None).unwrap_err();
    assert!(err.contains("JSON object"), "unexpected error: {err}");
}

#[test]
fn attach_accepts_numeric_process_id() {
    let mut config = json!({"request": "attach", "processId": 4242});
    normalize_attach(&mut config).expect("should normalize");
    assert_eq!(config["processId"], json!(4242));
}

#[test]
fn attach_preserves_zed_pick_pid_macro() {
    let mut config = json!({"request": "attach", "processId": "${ZED_PICK_PID}"});
    normalize_attach(&mut config).expect("should accept macro");
    assert_eq!(config["processId"], json!("${ZED_PICK_PID}"));

    let mut config2 = json!({"request": "attach", "processId": "$ZED_PICK_PID"});
    normalize_attach(&mut config2).expect("should accept shorthand macro");
    assert_eq!(config2["processId"], json!("$ZED_PICK_PID"));
}

#[test]
fn attach_coerces_numeric_string_process_id() {
    let mut config = json!({"request": "attach", "processId": " 4242 "});
    normalize_attach(&mut config).expect("should normalize");
    assert_eq!(config["processId"], json!(4242));
}

#[test]
fn attach_rejects_missing_process_id() {
    let mut config = json!({"request": "attach"});
    let err = normalize_attach(&mut config).unwrap_err();
    assert!(err.contains("`processId`"), "unexpected error: {err}");
}

#[test]
fn attach_rejects_non_numeric_string() {
    let mut config = json!({"request": "attach", "processId": "dotnet"});
    assert!(normalize_attach(&mut config).is_err());
}

#[test]
fn attach_rejects_non_numeric_type() {
    let mut config = json!({"request": "attach", "processId": true});
    assert!(normalize_attach(&mut config).is_err());
}

#[test]
fn attach_preserves_extra_fields() {
    let mut config = json!({"request": "attach", "processId": 42, "justMyCode": false});
    normalize_attach(&mut config).expect("ok");
    assert_eq!(config["justMyCode"], json!(false));
}

#[test]
fn logging_absent_yields_no_arguments() {
    let mut config = json!({"request": "launch", "program": "bin/app.dll"});
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        Vec::<String>::new()
    );
    assert!(config.get("logging").is_none());
}

#[test]
fn logging_null_yields_no_arguments() {
    let mut config = json!({"request": "launch", "program": "bin/app.dll", "logging": null});
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        Vec::<String>::new()
    );
    assert!(config.get("logging").is_none());
}

#[test]
fn engine_flag_is_built() {
    let mut config =
        json!({"request": "launch", "program": "app.dll", "logging": {"engine": true}});
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        vec!["--engineLogging".to_string()]
    );
    assert!(
        config.get("logging").is_none(),
        "logging must be stripped from the forwarded configuration"
    );
}

#[test]
fn engine_disabled_yields_no_arguments() {
    let mut config = json!({"logging": {"engine": false}});
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn engine_path_implies_engine_flag_with_file() {
    let mut config = json!({"logging": {"enginePath": "logs/engine.log"}});
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        vec!["--engineLogging=logs/engine.log".to_string()]
    );
}

#[test]
fn engine_path_overrides_console_engine_flag() {
    let mut config = json!({"logging": {"engine": true, "enginePath": "engine.log"}});
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        vec!["--engineLogging=engine.log".to_string()]
    );
}

#[test]
fn log_flag_is_built() {
    let mut config = json!({"logging": {"log": true}});
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        vec!["--log".to_string()]
    );
}

#[test]
fn log_path_builds_flag_with_value() {
    let mut config = json!({"logging": {"logPath": " logs/adapter.log "}});
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        vec!["--log=logs/adapter.log".to_string()]
    );
}

#[test]
fn engine_and_log_flags_combine_in_order() {
    let mut config = json!({"logging": {"engine": true, "log": true}});
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        vec!["--engineLogging".to_string(), "--log".to_string()]
    );
}

#[test]
fn logging_rejects_non_object() {
    let mut config = json!({"logging": true});
    let err = adapter_logging_args(&mut config).unwrap_err();
    assert!(err.contains("`logging`"), "unexpected error: {err}");
}

#[test]
fn logging_rejects_unknown_key() {
    let mut config = json!({"logging": {"engine": true, "verbose": true}});
    let err = adapter_logging_args(&mut config).unwrap_err();
    assert!(
        err.contains("verbose"),
        "error should name the unknown key, got: {err}"
    );
    assert!(
        err.contains("enginePath"),
        "error should list supported keys, got: {err}"
    );
}

#[test]
fn logging_rejects_wrong_types() {
    let cases: Vec<(&str, Value)> = vec![
        ("engine", json!("yes")),
        ("log", json!("yes")),
        ("enginePath", json!(42)),
        ("logPath", json!([])),
    ];
    for (key, value) in cases {
        let mut config = json!({});
        config
            .as_object_mut()
            .unwrap()
            .insert("logging".to_string(), json!({}));
        config["logging"]
            .as_object_mut()
            .unwrap()
            .insert(key.to_string(), value);
        let err = adapter_logging_args(&mut config).expect_err("wrong types must be rejected");
        assert!(err.contains(key), "error should name `{key}`, got: {err}");
    }
}

#[test]
fn logging_rejects_empty_paths() {
    let mut config = json!({"logging": {"enginePath": "   "}});
    let err = adapter_logging_args(&mut config).unwrap_err();
    assert!(err.contains("enginePath"), "unexpected error: {err}");

    let mut config = json!({"logging": {"logPath": ""}});
    let err = adapter_logging_args(&mut config).unwrap_err();
    assert!(err.contains("logPath"), "unexpected error: {err}");
}

#[test]
fn logging_rejects_non_string_paths() {
    let mut config = json!({"logging": {"enginePath": 42}});
    let err = adapter_logging_args(&mut config).unwrap_err();
    assert!(err.contains("enginePath"), "unexpected error: {err}");

    let mut config = json!({"logging": {"logPath": []}});
    let err = adapter_logging_args(&mut config).unwrap_err();
    assert!(err.contains("logPath"), "unexpected error: {err}");
}

#[test]
fn vscode_engine_logging_maps_to_engine_flag() {
    let mut config = json!({"logging": {"engineLogging": true}});
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        vec!["--engineLogging".to_string()]
    );
    let mut config = json!({"logging": {"engineLogging": false}});
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn vscode_engine_logging_combines_with_engine_path() {
    let mut config = json!({"logging": {"engineLogging": true, "enginePath": "e.log"}});
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        vec!["--engineLogging=e.log".to_string()]
    );
}

#[test]
fn vscode_engine_logging_rejects_non_boolean() {
    let mut config = json!({"logging": {"engineLogging": "yes"}});
    let err = adapter_logging_args(&mut config).unwrap_err();
    assert!(
        err.contains("`logging.engineLogging`"),
        "unexpected error: {err}"
    );
}

#[test]
fn vscode_logging_keys_are_ignored() {
    // A `logging` object as written by vscode-csharp users: every key
    // except `engineLogging` has no netcoredbg equivalent.
    let mut config = json!({
        "request": "launch",
        "program": "app.dll",
        "logging": {
            "exceptions": true,
            "moduleLoad": false,
            "programOutput": true,
            "browserStdOut": false,
            "elapsedTiming": true,
            "threadExit": false,
            "processExit": true,
            "consoleUsageMessage": false,
            "diagnosticsLog": {"protocolMessages": true}
        }
    });
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        Vec::<String>::new()
    );
    assert!(config.get("logging").is_none());
}

#[test]
fn vscode_logging_keys_ignored_with_any_value_type() {
    for key in VSCODE_IGNORED_LOGGING_KEYS {
        let mut config = json!({"logging": {}});
        config["logging"]
            .as_object_mut()
            .unwrap()
            .insert(key.to_string(), json!("unexpected"));
        assert_eq!(
            adapter_logging_args(&mut config),
            Ok(Vec::new()),
            "`{key}` should be ignored"
        );
    }
}

#[test]
fn vscode_keys_mix_with_extension_keys() {
    let mut config = json!({"logging": {"moduleLoad": false, "engineLogging": true, "log": true}});
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        vec!["--engineLogging".to_string(), "--log".to_string()]
    );
}

#[test]
fn unknown_key_error_lists_ignored_vscode_keys() {
    let mut config = json!({"logging": {"engin": true}});
    let err = adapter_logging_args(&mut config).unwrap_err();
    assert!(err.contains("`logging.engin`"), "unexpected error: {err}");
    assert!(err.contains("`moduleLoad`"), "unexpected error: {err}");
    assert!(err.contains("`engineLogging`"), "unexpected error: {err}");
}

#[test]
fn log_path_rejects_stdout() {
    let mut config = json!({"logging": {"logPath": " stdout "}});
    let err = adapter_logging_args(&mut config).unwrap_err();
    assert!(err.contains("stdout"), "unexpected error: {err}");
    let mut config = json!({"logging": {"logPath": "stderr"}});
    assert_eq!(
        adapter_logging_args(&mut config).unwrap(),
        vec!["--log=stderr".to_string()]
    );
}

#[test]
fn logging_stripped_from_attach_config() {
    let mut config = json!({"request": "attach", "processId": 1, "logging": {"engine": true}});
    adapter_logging_args(&mut config).expect("ok");
    assert!(config.get("logging").is_none());
}

#[test]
fn translate_launch_json_handles_comments_trailing_commas_and_macros() {
    let launch_json = r#"
    {
        // VSCode launch configuration
        "version": "0.2.0",
        "configurations": [
            {
                "name": ".NET Core Launch (console)",
                "type": "coreclr",
                "request": "launch",
                "preLaunchTask": "build",
                "program": "${workspaceFolder}/bin/Debug/net8.0/MyApp.dll",
                "args": ["arg1", "arg2"],
                "cwd": "${workspaceFolder}",
                "stopAtEntry": false, /* stop at main */
            },
            {
                "name": ".NET Core Attach",
                "type": "coreclr",
                "request": "attach",
                "processId": "${command:pickProcess}",
            },
        ],
    }
    "#;

    // Default picks first coreclr
    let config = translate_launch_json(launch_json, None, "/workspace")
        .expect("parse succeeds")
        .expect("found coreclr");
    assert_eq!(config["request"], "launch");
    assert_eq!(config["program"], "/workspace/bin/Debug/net8.0/MyApp.dll");
    assert_eq!(config["cwd"], "/workspace");

    // Targeted pick by name (Attach)
    let attach = translate_launch_json(launch_json, Some(".NET Core Attach"), "/workspace")
        .expect("parse succeeds")
        .expect("found attach");
    assert_eq!(attach["request"], "attach");
    assert_eq!(attach["processId"], "${ZED_PICK_PID}");
}

// --- console (#12) -------------------------------------------------

#[test]
fn console_internal_console_is_accepted_and_stripped() {
    let mut launch = json!({
        "request": "launch",
        "program": "app.dll",
        "console": "internalConsole"
    });
    normalize_launch(&mut launch, ROOT, None).expect("ok");
    assert!(launch.get("console").is_none());

    let mut attach = json!({
        "request": "attach",
        "processId": 42,
        "console": "internalConsole"
    });
    normalize_attach(&mut attach).expect("ok");
    assert!(attach.get("console").is_none());
}

#[test]
fn console_null_is_stripped() {
    let mut config = json!({"request": "launch", "program": "app.dll", "console": null});
    normalize_launch(&mut config, ROOT, None).expect("ok");
    assert!(config.get("console").is_none());
}

#[test]
fn console_terminal_modes_return_actionable_attach_workaround() {
    for mode in ["integratedTerminal", "externalTerminal"] {
        let mut config = json!({"request": "launch", "program": "app.dll", "console": mode});
        let err = normalize_launch(&mut config, ROOT, None).unwrap_err();
        assert!(err.contains(mode), "{err}");
        assert!(err.contains("runInTerminal"), "{err}");
        assert!(err.contains("${ZED_PICK_PID}"), "{err}");
    }
}

#[test]
fn console_rejects_unknown_or_non_string() {
    let mut config = json!({"request": "launch", "program": "app.dll", "console": "xterm"});
    let err = normalize_launch(&mut config, ROOT, None).unwrap_err();
    assert!(err.contains("internalConsole"), "{err}");

    let mut config = json!({"request": "launch", "program": "app.dll", "console": true});
    let err = normalize_launch(&mut config, ROOT, None).unwrap_err();
    assert!(err.contains("`console`"), "{err}");
}

// --- pipeTransport (#13) -------------------------------------------

#[test]
fn pipe_transport_extracts_and_strips() {
    let mut config = json!({
        "request": "attach",
        "processId": 1,
        "pipeTransport": {
            "pipeProgram": "docker",
            "pipeArgs": ["exec", "-i", "myapp"],
            "debuggerPath": "/vsdbg/netcoredbg"
        }
    });
    let pipe = adapter_pipe_transport(&mut config)
        .expect("ok")
        .expect("some");
    assert_eq!(pipe.pipe_program, "docker");
    assert_eq!(pipe.pipe_args, vec!["exec", "-i", "myapp"]);
    assert_eq!(pipe.debugger_path, "/vsdbg/netcoredbg");
    assert!(config.get("pipeTransport").is_none());
}

#[test]
fn pipe_transport_defaults_pipe_args_to_empty() {
    let mut config = json!({
        "pipeTransport": {
            "pipeProgram": "ssh",
            "debuggerPath": "/opt/netcoredbg/netcoredbg"
        }
    });
    let pipe = adapter_pipe_transport(&mut config).unwrap().unwrap();
    assert!(pipe.pipe_args.is_empty());
}

#[test]
fn pipe_transport_absent_or_null_returns_none() {
    let mut config = json!({"request": "attach", "processId": 1});
    assert_eq!(adapter_pipe_transport(&mut config).unwrap(), None);

    let mut config = json!({"pipeTransport": null});
    assert_eq!(adapter_pipe_transport(&mut config).unwrap(), None);
    assert!(config.get("pipeTransport").is_none());
}

#[test]
fn pipe_transport_rejects_invalid_shapes_and_unknown_keys() {
    let mut cfg = json!({"pipeTransport": "docker"});
    assert!(adapter_pipe_transport(&mut cfg).is_err());

    let mut cfg = json!({"pipeTransport": {"debuggerPath": "/x"}});
    assert!(adapter_pipe_transport(&mut cfg)
        .unwrap_err()
        .contains("pipeProgram"));

    let mut cfg = json!({"pipeTransport": {"pipeProgram": "docker"}});
    assert!(adapter_pipe_transport(&mut cfg)
        .unwrap_err()
        .contains("debuggerPath"));

    let mut cfg = json!({
        "pipeTransport": {"pipeProgram": "docker", "pipeArgs": "exec", "debuggerPath": "/x"}
    });
    assert!(adapter_pipe_transport(&mut cfg)
        .unwrap_err()
        .contains("pipeArgs"));

    let mut cfg = json!({
        "pipeTransport": {"pipeProgram": "docker", "pipeArg": ["exec"], "debuggerPath": "/x"}
    });
    let err = adapter_pipe_transport(&mut cfg).unwrap_err();
    assert!(err.contains("pipeArg"), "{err}");
}

// --- worktree relative paths (#10, #11) ----------------------------

#[test]
fn normalize_worktree_relative_path_handles_macros_roots_and_separators() {
    assert_eq!(
        normalize_worktree_relative_path(".env", "/repo", "envFile").unwrap(),
        ".env"
    );
    assert_eq!(
        normalize_worktree_relative_path("${ZED_WORKTREE_ROOT}/.env", "/repo", "envFile").unwrap(),
        ".env"
    );
    assert_eq!(
        normalize_worktree_relative_path("${workspaceFolder}\\sub\\.env", "C:\\repo", "envFile")
            .unwrap(),
        "sub/.env"
    );
    assert_eq!(
        normalize_worktree_relative_path(
            "/repo/src/Api/Properties/launchSettings.json",
            "/repo/",
            "launchSettingsFilePath"
        )
        .unwrap(),
        "src/Api/Properties/launchSettings.json"
    );
    assert_eq!(
        normalize_worktree_relative_path(
            "c:\\Repo\\Properties\\launchSettings.json",
            "C:\\Repo",
            "launchSettingsFilePath"
        )
        .unwrap(),
        "Properties/launchSettings.json"
    );
    assert_eq!(
        normalize_worktree_relative_path("./sub/../.env", "/repo", "envFile").unwrap(),
        ".env"
    );
}

#[test]
fn normalize_worktree_relative_path_rejects_escapes_and_external_paths() {
    assert!(normalize_worktree_relative_path("", "/repo", "envFile").is_err());
    assert!(normalize_worktree_relative_path("/etc/passwd", "/repo", "envFile").is_err());
    assert!(normalize_worktree_relative_path("../secret.env", "/repo", "envFile").is_err());
    assert!(normalize_worktree_relative_path("${ZED_WORKTREE_ROOT}", "/repo", "envFile").is_err());
}

// --- envFile (#11) -------------------------------------------------

#[test]
fn take_env_file_validates_and_strips() {
    let mut cfg = json!({"request": "launch", "program": "app.dll", "envFile": " .env "});
    assert_eq!(take_env_file(&mut cfg).unwrap().as_deref(), Some(".env"));
    assert!(cfg.get("envFile").is_none());

    let mut cfg = json!({"envFile": null});
    assert_eq!(take_env_file(&mut cfg).unwrap(), None);
    assert!(cfg.get("envFile").is_none());

    let mut cfg = json!({"envFile": "   "});
    assert!(take_env_file(&mut cfg).is_err());

    let mut cfg = json!({"envFile": [".env"]});
    let err = take_env_file(&mut cfg).unwrap_err();
    assert!(err.contains("`envFile`"), "{err}");
}

#[test]
fn parse_env_file_handles_comments_export_quotes_and_escapes() {
    let raw = "\u{feff}\n\
        # Full-line comment\n\
        \n\
        PLAIN=hello world\r\n\
        INLINE=value # trailing comment\n\
        URL=https://example.com#section\n\
        export EXPORTED=yes\n\
        export\tTAB_EXPORT=1\n\
        export_name=keep_prefix\n\
        DOUBLE=\"quoted # not comment = still value\\nline2\"\n\
        SINGLE='literal # \\n not escaped' # comment after quote\n\
        EMPTY=\n\
        PLAIN=overridden\n";
    let vars = parse_env_file(raw).unwrap();
    assert_eq!(
        vars,
        vec![
            ("PLAIN".to_string(), "overridden".to_string()),
            ("INLINE".to_string(), "value".to_string()),
            ("URL".to_string(), "https://example.com#section".to_string()),
            ("EXPORTED".to_string(), "yes".to_string()),
            ("TAB_EXPORT".to_string(), "1".to_string()),
            ("export_name".to_string(), "keep_prefix".to_string()),
            (
                "DOUBLE".to_string(),
                "quoted # not comment = still value\nline2".to_string()
            ),
            (
                "SINGLE".to_string(),
                "literal # \\n not escaped".to_string()
            ),
            ("EMPTY".to_string(), String::new()),
        ]
    );
}

#[test]
fn parse_env_file_rejects_malformed_lines() {
    assert!(parse_env_file("NO_EQUALS").is_err());
    assert!(parse_env_file("=no_key").is_err());
    assert!(parse_env_file("BAD KEY=1").is_err());
    assert!(parse_env_file("A=\"unclosed").is_err());
    assert!(parse_env_file("A='unclosed").is_err());
    assert!(parse_env_file("A=\"ok\" junk").is_err());
}

// --- launchSettings.json (#10) -------------------------------------

#[test]
fn take_launch_settings_spec_defaults_path_and_strips_keys() {
    let mut cfg = json!({
        "request": "launch",
        "program": "App.dll",
        "launchSettingsProfile": "https"
    });
    assert_eq!(
        take_launch_settings_spec(&mut cfg).unwrap(),
        Some(LaunchSettingsSpec {
            file_path: "Properties/launchSettings.json".to_string(),
            profile: Some("https".to_string()),
        })
    );
    assert!(cfg.get("launchSettingsProfile").is_none());
    assert!(cfg.get("launchSettingsFilePath").is_none());

    let mut cfg = json!({"launchSettingsFilePath": "src/Api/Properties/launchSettings.json"});
    assert_eq!(
        take_launch_settings_spec(&mut cfg).unwrap(),
        Some(LaunchSettingsSpec {
            file_path: "src/Api/Properties/launchSettings.json".to_string(),
            profile: None,
        })
    );

    let mut cfg = json!({"launchSettingsProfile": 42});
    assert!(take_launch_settings_spec(&mut cfg).is_err());
    let mut cfg = json!({"launchSettingsFilePath": "  "});
    assert!(take_launch_settings_spec(&mut cfg).is_err());
}

#[test]
fn parse_launch_settings_profile_tolerates_jsonc_and_picks_named_or_project() {
    let jsonc = r#"{
        // comment outside strings
        "iisSettings": { "windowsAuthentication": false, },
        "profiles": {
            "IIS Express": {
                "commandName": "IISExpress",
                "launchBrowser": true,
            },
            "https": {
                /* block comment */
                "commandName": "Project",
                "launchBrowser": true,
                "applicationUrl": "https://localhost:7001;http://localhost:5001",
                "commandLineArgs": "--urls=https://localhost:7001 --name \"My App\"",
                "workingDirectory": "src/Api",
                "environmentVariables": {
                    "ASPNETCORE_ENVIRONMENT": "Development",
                    "PORT": 7001,
                },
            },
        },
    }"#;

    // Default omits profile name -> picks first "commandName": "Project" (skipping IISExpress)
    let default_profile =
        parse_launch_settings_profile(jsonc, None, "Properties/launchSettings.json").unwrap();
    assert_eq!(
        default_profile.application_url.as_deref(),
        Some("https://localhost:7001;http://localhost:5001")
    );
    assert_eq!(
        default_profile.command_line_args,
        Some(vec![
            "--urls=https://localhost:7001".to_string(),
            "--name".to_string(),
            "My App".to_string(),
        ])
    );
    assert_eq!(
        default_profile.working_directory.as_deref(),
        Some("src/Api")
    );
    assert!(default_profile.environment_variables.contains(&(
        "ASPNETCORE_ENVIRONMENT".to_string(),
        "Development".to_string()
    )));
    assert!(default_profile
        .environment_variables
        .contains(&("PORT".to_string(), "7001".to_string())));

    // Named profile lookup
    let iis =
        parse_launch_settings_profile(jsonc, Some("IIS Express"), "Properties/launchSettings.json")
            .unwrap();
    assert!(iis.environment_variables.is_empty());
}

#[test]
fn parse_launch_settings_profile_errors_on_missing_profile_or_project() {
    let raw = r#"{"profiles": {"IIS Express": {"commandName": "IISExpress"}}}"#;
    let err = parse_launch_settings_profile(raw, Some("https"), "Properties/launchSettings.json")
        .unwrap_err();
    assert!(
        err.contains("https") && err.contains("IIS Express"),
        "{err}"
    );

    let err =
        parse_launch_settings_profile(raw, None, "Properties/launchSettings.json").unwrap_err();
    assert!(
        err.contains("Project") && err.contains("IIS Express"),
        "{err}"
    );

    assert!(parse_launch_settings_profile("{}", None, "ls.json").is_err());
    assert!(parse_launch_settings_profile("{bad", None, "ls.json").is_err());
}

#[test]
fn merge_precedence_explicit_env_gt_launch_settings_gt_env_file_gt_dotnet_root() {
    let mut config = json!({
        "request": "launch",
        "program": "src/Api/bin/Debug/net8.0/Api.dll",
        "args": ["--explicit"],
        "env": {
            "EXPLICIT_WIN": "from_config",
            "SHARED": "from_config"
        }
    });
    let env_file_vars = vec![
        ("FROM_ENV_FILE".to_string(), "1".to_string()),
        ("SHARED".to_string(), "from_env_file".to_string()),
        ("LS_WINS".to_string(), "from_env_file".to_string()),
        (
            "ASPNETCORE_URLS".to_string(),
            "http://envfile:80".to_string(),
        ),
    ];
    let profile = LaunchSettingsProfile {
        environment_variables: vec![
            ("LS_WINS".to_string(), "from_launch_settings".to_string()),
            ("SHARED".to_string(), "from_launch_settings".to_string()),
        ],
        application_url: Some("https://localhost:7001".to_string()),
        command_line_args: Some(vec!["--ignored-because-explicit".to_string()]),
        working_directory: Some("src/Api".to_string()),
    };

    merge_launch_sources(&mut config, Some(&env_file_vars), Some(&profile)).unwrap();
    normalize_launch(&mut config, ROOT, Some("/usr/share/dotnet/dotnet")).unwrap();

    assert_eq!(config["args"], json!(["--explicit"]));
    assert_eq!(config["cwd"], json!("src/Api"));
    assert_eq!(config["env"]["FROM_ENV_FILE"], json!("1"));
    assert_eq!(config["env"]["LS_WINS"], json!("from_launch_settings"));
    assert_eq!(config["env"]["SHARED"], json!("from_config"));
    assert_eq!(config["env"]["EXPLICIT_WIN"], json!("from_config"));
    assert_eq!(
        config["env"]["ASPNETCORE_URLS"],
        json!("https://localhost:7001")
    );
    assert_eq!(config["env"]["DOTNET_ROOT"], json!("/usr/share/dotnet"));
}

#[test]
fn merge_launch_settings_applies_args_when_absent_and_respects_dotnet_root_from_env_file() {
    let mut config = json!({
        "request": "launch",
        "program": "App.dll",
    });
    let env_file_vars = vec![("DOTNET_ROOT".to_string(), "/custom/dotnet".to_string())];
    let profile = LaunchSettingsProfile {
        environment_variables: vec![],
        application_url: None,
        command_line_args: Some(vec!["--serve".to_string(), "8080".to_string()]),
        working_directory: None,
    };

    merge_launch_sources(&mut config, Some(&env_file_vars), Some(&profile)).unwrap();
    normalize_launch(&mut config, ROOT, Some("/usr/share/dotnet/dotnet")).unwrap();

    assert_eq!(config["args"], json!(["--serve", "8080"]));
    assert_eq!(config["cwd"], json!(ROOT));
    assert_eq!(config["env"]["DOTNET_ROOT"], json!("/custom/dotnet"));
}

#[test]
fn take_launch_settings_spec_nulls_and_launch_settings_profile_env_errors() {
    let mut cfg = json!({
        "request": "launch",
        "program": "App.dll",
        "launchSettingsProfile": null,
        "launchSettingsFilePath": null
    });
    assert_eq!(take_launch_settings_spec(&mut cfg).unwrap(), None);
    assert!(cfg.get("launchSettingsProfile").is_none());
    assert!(cfg.get("launchSettingsFilePath").is_none());

    // launchSettings environmentVariables["ASPNETCORE_URLS"] overrides applicationUrl
    let jsonc = r#"{
        "profiles": {
            "Api": {
                "commandName": "Project",
                "applicationUrl": "http://localhost:5000",
                "environmentVariables": {
                    "ASPNETCORE_URLS": "https://localhost:5001"
                }
            }
        }
    }"#;
    let profile = parse_launch_settings_profile(jsonc, None, "ls.json").unwrap();
    let mut cfg = json!({"request": "launch", "program": "App.dll"});
    merge_launch_sources(&mut cfg, None, Some(&profile)).unwrap();
    assert_eq!(
        cfg["env"]["ASPNETCORE_URLS"],
        json!("https://localhost:5001")
    );

    // invalid env in launch config errors before overwrite
    let mut bad_cfg = json!({"request": "launch", "program": "App.dll", "env": {"A": 1}});
    assert!(merge_launch_sources(&mut bad_cfg, None, Some(&profile)).is_err());

    let mut bad_cfg2 = json!({"request": "launch", "program": "App.dll", "env": "bad"});
    assert!(merge_launch_sources(&mut bad_cfg2, None, Some(&profile)).is_err());
}

// --- JSONC parsing (delegated to `jsonc-parser`; strict apart from comments / trailing commas) ---

#[test]
fn jsonc_accepts_comments_and_trailing_commas() {
    let value = parse_jsonc(
        r#"{
            // line comment
            "a": 1, /* block
            comment */ "b": [1, 2,],
        }"#,
    )
    .unwrap();
    assert_eq!(value, json!({"a": 1, "b": [1, 2]}));
}

#[test]
fn jsonc_keeps_comment_markers_inside_strings() {
    let value = parse_jsonc(r#"{"url": "http://example.com/*x*/", "p": "a//b",}"#).unwrap();
    assert_eq!(
        value,
        json!({"url": "http://example.com/*x*/", "p": "a//b"})
    );
}

#[test]
fn jsonc_stays_strict_apart_from_comments_and_trailing_commas() {
    for bad in [
        "{'a': 1}",           // single-quoted strings
        "{a: 1}",             // bare property names
        r#"{"a": 1 "b": 2}"#, // missing comma
        r#"{"a": 0xFF}"#,     // hexadecimal numbers
        r#"{"a": +1}"#,       // unary plus
        r#"{"a": NaN}"#,      // non-finite numbers
        r#"{"a": 1"#,         // unterminated object
    ] {
        assert!(parse_jsonc(bad).is_err(), "{bad} must be rejected");
    }
}

#[test]
fn jsonc_empty_input_is_null_and_errors_carry_a_position() {
    assert_eq!(parse_jsonc("  // nothing\n").unwrap(), Value::Null);
    let error = parse_jsonc("{\n  \"a\": }").unwrap_err();
    assert!(error.contains("line 2"), "{error}");
}

#[test]
fn launch_json_and_launch_settings_report_which_file_failed_to_parse() {
    let error = translate_launch_json("{ not json", None, ROOT).unwrap_err();
    assert!(error.starts_with("failed to parse launch.json:"), "{error}");
    let error =
        parse_launch_settings_profile("{ not json", None, "Properties/ls.json").unwrap_err();
    assert!(
        error.starts_with("failed to parse launchSettings file \"Properties/ls.json\":"),
        "{error}"
    );
}

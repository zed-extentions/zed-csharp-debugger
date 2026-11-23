//! csharp-debugger — run and debug C#/.NET applications in Zed via
//! netcoredbg (https://github.com/Samsung/netcoredbg, MIT) speaking the
//! Debug Adapter Protocol over stdio.
//!
//! Reimplements the launch/attach wiring that the VSCode C# extension
//! performs, against the Zed extension API. The adapter binary is resolved
//! in order: user setting (`dap.netcoredbg.binary`) → `PATH` probe →
//! automatic download from GitHub Releases (FR-16) → instructive error.
//!
//! Zed entry points (FR-19): `dap_request_kind` tells Zed whether a
//! `.zed/debug.json` scenario launches or attaches (without it Zed rejects
//! every scenario), and `dap_config_to_scenario` turns the new-process
//! modal's debugger-agnostic request into our adapter configuration.
//!
//! A `dotnet build` debug locator (FR-17) turns `dotnet build` tasks into
//! debug scenarios: phase 1 (`dap_locator_create_scenario`) attaches the
//! build task to a launch scenario, phase 2 (`run_dap_locator`) asks MSBuild
//! for the build output (`TargetPath`) after the build finishes, and
//! `dap_config_to_scenario` produces the final adapter configuration.

mod config;
mod download;
mod locator;
mod resolve;

pub(crate) use zed_extension_api::serde_json;

use std::path::Path;
use zed_extension_api::process::Command;
use zed_extension_api::{
    self as zed, current_platform, Architecture, AttachRequest, BuildTaskDefinition,
    BuildTaskDefinitionTemplatePayload, DebugAdapterBinary, DebugConfig, DebugRequest,
    DebugScenario, DebugTaskDefinition, LaunchRequest, Os, StartDebuggingRequestArguments,
    StartDebuggingRequestArgumentsRequest, TaskTemplate, Worktree,
};
use zed_extension_api::{DownloadedFileType, GithubReleaseOptions};

#[derive(Default)]
struct NetcoredbgExtension {
    /// The downloaded adapter binary resolved earlier in this Zed session
    /// (FR-16), so the GitHub API is queried at most once per session.
    downloaded_binary: Option<String>,
}

impl zed::Extension for NetcoredbgExtension {
    fn new() -> Self {
        Self::default()
    }

    fn get_dap_binary(
        &mut self,
        adapter_name: String,
        definition: DebugTaskDefinition,
        user_provided_debug_adapter_path: Option<String>,
        worktree: &Worktree,
    ) -> Result<DebugAdapterBinary, String> {
        if adapter_name != resolve::ADAPTER_NAME {
            return Err(format!(
                "unknown debug adapter `{adapter_name}`; this extension provides `{}`",
                resolve::ADAPTER_NAME
            ));
        }

        let mut json_config: serde_json::Value = serde_json::from_str(&definition.config)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        let root = worktree.root_path();

        // If the configuration lacks a request or program/processId, attempt to resolve and bridge
        // a coreclr configuration from .vscode/launch.json using the Worktree API.
        let needs_fallback = match config::request_kind(&json_config) {
            Err(_) => true,
            Ok(config::RequestKind::Launch) => json_config.get("program").is_none(),
            Ok(config::RequestKind::Attach) => json_config.get("processId").is_none(),
        };

        if needs_fallback {
            if let Ok(launch_content) = worktree.read_text_file(".vscode/launch.json") {
                let target_name = json_config.get("name").and_then(serde_json::Value::as_str);
                if let Ok(Some(translated)) =
                    config::translate_launch_json(&launch_content, target_name, &root)
                {
                    if let Some(target_obj) = translated.as_object() {
                        if let Some(cfg) = json_config.as_object_mut() {
                            for (k, v) in target_obj {
                                cfg.entry(k.clone()).or_insert_with(|| v.clone());
                            }
                        } else {
                            json_config = translated;
                        }
                    }
                }
            }
        }

        // The diagnostics `logging` object maps to netcoredbg *adapter*
        // command line flags, and `pipeTransport` wraps adapter execution via
        // an external pipe command; both are stripped before forwarding DAP args.
        let logging_args = config::adapter_logging_args(&mut json_config)?;
        let pipe_transport = config::adapter_pipe_transport(&mut json_config)?;
        if definition.tcp_connection.is_some() && pipe_transport.is_some() {
            return Err(
                "cannot combine `tcp_connection` and `pipeTransport`: use either TCP \
                 connect (`tcp_connection`) or `pipeTransport`"
                    .to_string(),
            );
        }

        let env_file_raw = config::take_env_file(&mut json_config)?;
        let launch_settings_spec = config::take_launch_settings_spec(&mut json_config)?;

        let is_remote = definition.tcp_connection.is_some() || pipe_transport.is_some();
        let dotnet_path = if is_remote {
            None
        } else {
            worktree.which("dotnet")
        };

        // The DOTNET_ROOT hint is injected both into the debuggee `env` (via
        // normalize_launch) and into the local adapter process environment: on
        // Windows netcoredbg itself needs it to locate the runtime. Remote TCP
        // and pipeTransport targets run outside the local host, so local
        // DOTNET_ROOT discovery is skipped for those transports.
        let mut envs: Vec<(String, String)> = Vec::new();
        if !is_remote {
            if let Some(hint) = resolve::dotnet_root_hint(&envs, dotnet_path.as_deref()) {
                envs.push(hint);
            }
        }

        let request = match config::request_kind(&json_config)? {
            config::RequestKind::Launch => {
                let env_file_vars = match env_file_raw {
                    Some(raw_path) => {
                        let rel =
                            config::normalize_worktree_relative_path(&raw_path, &root, "envFile")?;
                        let content = worktree.read_text_file(&rel).map_err(|error| {
                            format!("failed to read `envFile` {rel:?} in worktree: {error}")
                        })?;
                        Some(config::parse_env_file(&content)?)
                    }
                    None => None,
                };
                let launch_profile = match launch_settings_spec {
                    Some(spec) => {
                        let rel = config::normalize_worktree_relative_path(
                            &spec.file_path,
                            &root,
                            "launchSettingsFilePath",
                        )?;
                        let content = worktree.read_text_file(&rel).map_err(|error| {
                            format!(
                                "failed to read launchSettings file {rel:?} in worktree: {error}"
                            )
                        })?;
                        Some(config::parse_launch_settings_profile(
                            &content,
                            spec.profile.as_deref(),
                            &rel,
                        )?)
                    }
                    None => None,
                };
                config::merge_launch_sources(
                    &mut json_config,
                    env_file_vars.as_deref(),
                    launch_profile.as_ref(),
                )?;
                config::normalize_launch(&mut json_config, &root, dotnet_path.as_deref())?;
                StartDebuggingRequestArgumentsRequest::Launch
            }
            config::RequestKind::Attach => {
                if env_file_raw.is_some() {
                    return Err(
                        "`envFile` is only supported for \"launch\" configurations".to_string()
                    );
                }
                if launch_settings_spec.is_some() {
                    return Err(
                        "`launchSettingsProfile` and `launchSettingsFilePath` are only \
                         supported for \"launch\" configurations"
                            .to_string(),
                    );
                }
                config::normalize_attach(&mut json_config)?;
                StartDebuggingRequestArgumentsRequest::Attach
            }
        };

        let request_args = StartDebuggingRequestArguments {
            configuration: serde_json::to_string(&json_config)
                .map_err(|error| format!("failed to re-encode configuration: {error}"))?,
            request,
        };

        if let Some(tcp_template) = definition.tcp_connection {
            let tcp_args = zed::resolve_tcp_template(tcp_template)?;
            return Ok(DebugAdapterBinary {
                command: None,
                arguments: Vec::new(),
                envs: Vec::new(),
                cwd: None,
                connection: Some(tcp_args),
                request_args,
            });
        }

        if let Some(pipe) = pipe_transport {
            let adapter = resolve::resolve_pipe_transport_command(&pipe, &logging_args)?;
            return Ok(DebugAdapterBinary {
                command: Some(adapter.command),
                arguments: adapter.arguments,
                envs: Vec::new(),
                cwd: Some(root),
                connection: None,
                request_args,
            });
        }

        let (host_os, host_arch) = current_platform();
        let platform = match host_os {
            Os::Windows => resolve::Platform::Windows,
            _ => resolve::Platform::Unix,
        };
        let which_result = resolve::binary_candidates(platform)
            .iter()
            .find_map(|name| worktree.which(name));

        // Resolution order: ① user setting ② PATH probe ③ automatic download
        // (FR-16) ④ instructive error (kept as the offline fallback). The
        // diagnostics flags (FR-18) are appended whatever the source.
        let adapter = resolve::resolve_adapter_command_with_download(
            user_provided_debug_adapter_path.as_deref(),
            which_result.as_deref(),
            || self.ensure_downloaded_binary(host_os, host_arch, &worktree.shell_env()),
            &logging_args,
        )?;

        Ok(DebugAdapterBinary {
            command: Some(adapter.command),
            arguments: adapter.arguments,
            envs,
            cwd: Some(root),
            connection: None,
            request_args,
        })
    }

    /// FR-19: Zed asks this before every session. When no build/locator step is
    /// attached, an error rejects the scenario; when a locator build task is
    /// attached, returning `Err` tells Zed's `running.rs` that the phase-1
    /// placeholder config still needs phase-2 resolution via `run_dap_locator`.
    fn dap_request_kind(
        &mut self,
        adapter_name: String,
        config: serde_json::Value,
    ) -> Result<StartDebuggingRequestArgumentsRequest, String> {
        if adapter_name != resolve::ADAPTER_NAME {
            return Err(format!(
                "unknown debug adapter `{adapter_name}`; this extension provides `{}`",
                resolve::ADAPTER_NAME
            ));
        }
        Ok(match config::request_kind(&config)? {
            config::RequestKind::Launch => StartDebuggingRequestArgumentsRequest::Launch,
            config::RequestKind::Attach => StartDebuggingRequestArgumentsRequest::Attach,
        })
    }

    /// FR-19: converts a debugger-agnostic request (new-process modal, or a
    /// locator's phase 2) into our adapter configuration.
    fn dap_config_to_scenario(&mut self, config: DebugConfig) -> Result<DebugScenario, String> {
        let request = match config.request {
            DebugRequest::Launch(LaunchRequest {
                program,
                cwd,
                args,
                envs,
            }) => config::GenericRequest::Launch {
                program,
                cwd,
                args,
                envs,
            },
            DebugRequest::Attach(AttachRequest { process_id }) => {
                config::GenericRequest::Attach { process_id }
            }
        };
        let adapter_config = config::scenario_config(&request, config.stop_on_entry);

        Ok(DebugScenario {
            label: config.label,
            adapter: config.adapter,
            // No build step: a locator's build (if any) has already run, and
            // filling `build` here would make Zed run it again.
            build: None,
            config: serde_json::to_string(&adapter_config)
                .map_err(|error| format!("failed to encode configuration: {error}"))?,
            tcp_connection: None,
        })
    }

    /// Phase 1 of the `dotnet build` / `dotnet test` locator: offers a debug
    /// scenario for `dotnet build` and `dotnet test` tasks, carrying the build
    /// task itself (`dotnet test` is rewritten to `dotnet build` so failing
    /// tests do not abort the pre-launch build).
    fn dap_locator_create_scenario(
        &mut self,
        locator_name: String,
        mut build_task: TaskTemplate,
        resolved_label: String,
        _debug_adapter_name: String,
    ) -> Option<DebugScenario> {
        if locator_name != locator::LOCATOR_NAME
            || locator::dotnet_build_args(&build_task.command, &build_task.args).is_none()
        {
            // Bail out cheaply on tasks we cannot handle.
            return None;
        }

        if let Some((cmd, args)) =
            locator::prepare_test_task_for_build(&build_task.command, &build_task.args)
        {
            build_task.command = cmd;
            build_task.args = args;
        }

        Some(DebugScenario {
            label: resolved_label,
            adapter: resolve::ADAPTER_NAME.to_string(),
            build: Some(BuildTaskDefinition::Template(
                BuildTaskDefinitionTemplatePayload {
                    locator_name: Some(locator_name),
                    template: build_task,
                },
            )),
            config: locator::scenario_config(),
            tcp_connection: None,
        })
    }

    /// Phase 2 of the `dotnet build` locator: the build finished, locate the
    /// built program by asking MSBuild for `TargetPath` with the task's
    /// output-affecting switches forwarded.
    fn run_dap_locator(
        &mut self,
        locator_name: String,
        build_task: TaskTemplate,
    ) -> Result<DebugRequest, String> {
        if locator_name != locator::LOCATOR_NAME {
            return Err(format!(
                "unknown debug locator `{locator_name}`; this extension provides `{}`",
                locator::LOCATOR_NAME
            ));
        }
        let build_args = locator::dotnet_build_args(&build_task.command, &build_task.args)
            .ok_or_else(|| {
                format!(
                    "the `{}` locator only handles `dotnet build` or `dotnet test` tasks, got `{} {}`",
                    locator::LOCATOR_NAME,
                    build_task.command,
                    build_task.args.join(" ")
                )
            })?;

        let task_cwd = build_task.cwd.clone().unwrap_or_default();
        let parsed = locator::parse_dotnet_build_args(&build_args)?;
        let query_args = locator::msbuild_query_args(&parsed, &task_cwd)?;

        // `dotnet msbuild <target> -getProperty:...` only evaluates the
        // project (requires .NET SDK 8+). Needs the `process:exec`
        // capability for `dotnet msbuild **` declared in extension.toml.
        // `run_command` has no cwd parameter, so every path is absolute.
        let output = Command::new("dotnet")
            .args(query_args)
            .envs(build_task.env.clone())
            .output()
            .map_err(|error| {
                format!("failed to run `dotnet msbuild` to locate the build output: {error}")
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if output.status != Some(0) {
            return Err(locator::msbuild_failure_message(
                output.status,
                &stdout,
                &String::from_utf8_lossy(&output.stderr),
            ));
        }

        let properties = locator::parse_msbuild_properties(&stdout)?;
        let resolved = locator::resolve_program(&properties, &task_cwd)?;

        Ok(DebugRequest::Launch(LaunchRequest {
            program: resolved.program,
            cwd: Some(resolved.cwd),
            // A `dotnet build` task's args are build switches, not program
            // arguments.
            args: Vec::new(),
            envs: build_task.env,
        }))
    }
}

impl NetcoredbgExtension {
    /// Returns the command path of a downloaded netcoredbg, downloading it
    /// from GitHub Releases into the extension working directory if needed.
    ///
    /// Only called when neither the user setting nor a `PATH` probe produced
    /// a binary. The resolved path is memoized for the rest of the Zed
    /// session, so the GitHub API is queried at most once per session (the
    /// unauthenticated API is rate limited). When the release lookup or the
    /// download fails (offline, asset gap, HTTP error), the newest cached
    /// version is used instead, if any.
    fn ensure_downloaded_binary(
        &mut self,
        os: Os,
        arch: Architecture,
        shell_env: &[(String, String)],
    ) -> Result<String, String> {
        let host_os = match os {
            Os::Mac => download::HostOs::Mac,
            Os::Linux => download::HostOs::Linux,
            Os::Windows => download::HostOs::Windows,
        };
        let host_arch = match arch {
            Architecture::Aarch64 => download::HostArch::Aarch64,
            Architecture::X86 => download::HostArch::X86,
            Architecture::X8664 => download::HostArch::X86_64,
        };
        let force_update = download::force_update(shell_env);

        if let Some(path) =
            download::usable_memo(self.downloaded_binary.as_deref(), force_update, |path| {
                Path::new(path).is_file()
            })
        {
            return Ok(path);
        }

        // The WASI current directory is the extension working directory
        // (preopened by Zed); it is the only place we may read/write files.
        let extension_dir = std::env::current_dir().map_err(|error| {
            format!("failed to resolve the extension working directory: {error}")
        })?;

        let path = match download_release(&extension_dir, host_os, host_arch, force_update) {
            Ok(path) => path,
            Err(download_error) => {
                let cached = cached_versions(&extension_dir, host_os)
                    .and_then(|versions| download::newest_version(&versions))
                    .ok_or(download_error)?;
                extension_dir
                    .join(download::binary_relative_path(&cached, host_os)?)
                    .to_string_lossy()
                    .into_owned()
            }
        };
        self.downloaded_binary = Some(path.clone());
        Ok(path)
    }
}

/// Resolves the release to use, downloads and extracts its asset for this
/// platform (unless that version is already cached) and removes older
/// cached versions after a successful download.
fn download_release(
    extension_dir: &Path,
    host_os: download::HostOs,
    host_arch: download::HostArch,
    force_update: bool,
) -> Result<String, String> {
    let expected_asset = download::asset_name(host_os, host_arch)
        .ok_or_else(|| download::unsupported_platform_error(host_os, host_arch))?;

    // Pinned tag when configured, latest release otherwise.
    let release = match download::VERSION_PIN {
        Some(tag) => zed::github_release_by_tag_name(download::REPO, tag)?,
        None => zed::latest_github_release(
            download::REPO,
            GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?,
    };

    // Samsung dropped some platforms from newer releases (3.2.0 has no
    // `osx-amd64`). The extension API cannot list older releases, so fall
    // back to the known last release that still has the asset.
    let (release, asset) = match select_release_asset(&release, expected_asset) {
        Ok(asset) => (release, asset),
        Err(error) => {
            let tag = download::fallback_tag(host_os, host_arch)
                .filter(|tag| *tag != release.version)
                .ok_or(error)?;
            let fallback = zed::github_release_by_tag_name(download::REPO, tag)?;
            let asset = select_release_asset(&fallback, expected_asset)?;
            (fallback, asset)
        }
    };

    let plan = download::download_plan(&release.version, &asset.name, host_os)?;
    let binary_path = extension_dir.join(&plan.binary);
    if binary_path.is_file() && !force_update {
        return Ok(binary_path.to_string_lossy().into_owned());
    }

    // Start from an empty version directory (a previous attempt may have
    // been interrupted half-way through the extraction).
    let _ = std::fs::remove_dir_all(extension_dir.join(&plan.destination));
    // `download_file` extracts archives *into* `plan.destination`.
    zed::download_file(
        &asset.download_url,
        &plan.destination,
        match plan.kind {
            download::ArchiveKind::Gzip => DownloadedFileType::Gzip,
            download::ArchiveKind::GzipTar => DownloadedFileType::GzipTar,
            download::ArchiveKind::Zip => DownloadedFileType::Zip,
            download::ArchiveKind::Uncompressed => DownloadedFileType::Uncompressed,
        },
    )?;
    if !binary_path.is_file() {
        return Err(format!(
            "downloaded {} but the adapter binary was not found at {}",
            asset.name, plan.binary
        ));
    }
    // Windows binaries do not need an executable bit, but `chmod +x` on a
    // Windows path is a harmless host no-op and keeps this branch
    // host-agnostic (the WASM target has no usable `cfg!(target_os)`).
    zed::make_file_executable(&plan.binary)?;

    // Keep only the version just installed.
    if let Ok(entries) = std::fs::read_dir(extension_dir.join(download::CACHE_DIR_NAME)) {
        let dirs: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        for stale in download::stale_versions(&dirs, &release.version) {
            if let Ok(dir) = download::cache_dir_relative(&stale) {
                let _ = std::fs::remove_dir_all(extension_dir.join(dir));
            }
        }
    }

    Ok(binary_path.to_string_lossy().into_owned())
}

/// Selects this platform's asset from a GitHub release.
fn select_release_asset(
    release: &zed::GithubRelease,
    expected_asset: &str,
) -> Result<download::ReleaseAsset, String> {
    let assets: Vec<download::ReleaseAsset> = release
        .assets
        .iter()
        .map(|asset| download::ReleaseAsset {
            name: asset.name.clone(),
            download_url: asset.download_url.clone(),
        })
        .collect();
    download::select_asset(&assets, expected_asset).cloned()
}

/// Lists netcoredbg versions already cached in the extension working
/// directory whose extracted binary still exists (offline fallback).
fn cached_versions(extension_dir: &Path, os: download::HostOs) -> Option<Vec<String>> {
    let entries = std::fs::read_dir(extension_dir.join(download::CACHE_DIR_NAME)).ok()?;
    let mut versions = Vec::new();
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Ok(binary) = download::binary_relative_path(&name, os) {
                if extension_dir.join(binary).is_file() {
                    versions.push(name);
                }
            }
        }
    }
    Some(versions)
}

zed::register_extension!(NetcoredbgExtension);

#[cfg(test)]
mod manifest_tests {
    #[test]
    fn extension_manifest_is_registry_compliant() {
        let manifest = include_str!("../extension.toml");
        assert!(manifest.contains("id = \"csharp-debugger\""));
        assert!(manifest.contains("authors = [\"wruwami\"]"));
        assert!(!manifest.contains("[slash_commands"));
    }
}

/// FR-14: invariants of `debug_adapter_schemas/netcoredbg.json` that the G5 manual
/// procedure (steps 16, 24) used to check by eye in the editor.
#[cfg(test)]
mod schema_tests {
    use zed_extension_api::serde_json::{self, Value};

    fn schema() -> Value {
        serde_json::from_str(include_str!("../debug_adapter_schemas/netcoredbg.json"))
            .expect("schema is valid JSON")
    }

    fn variant<'a>(schema: &'a Value, request: &str) -> &'a Value {
        schema["oneOf"]
            .as_array()
            .expect("oneOf is an array")
            .iter()
            .find(|v| v["properties"]["request"]["const"] == request)
            .unwrap_or_else(|| panic!("no `{request}` variant"))
    }

    fn props(variant: &Value) -> &serde_json::Map<String, Value> {
        variant["properties"].as_object().expect("properties")
    }

    fn required(variant: &Value) -> Vec<&str> {
        variant["required"]
            .as_array()
            .expect("required")
            .iter()
            .filter_map(Value::as_str)
            .collect()
    }

    #[test]
    fn has_exactly_a_launch_and_an_attach_variant() {
        let schema = schema();
        assert_eq!(schema["oneOf"].as_array().map(Vec::len), Some(2));
        assert_eq!(required(variant(&schema, "launch")), ["request", "program"]);
        assert_eq!(
            required(variant(&schema, "attach")),
            ["request", "processId"]
        );
    }

    #[test]
    fn launch_only_options_are_absent_from_attach() {
        // netcoredbg reads only `processId` on attach, so these must not be completed.
        let schema = schema();
        let launch = props(variant(&schema, "launch"));
        let attach = props(variant(&schema, "attach"));
        for key in [
            "justMyCode",
            "enableStepFiltering",
            "stopAtEntry",
            "program",
        ] {
            assert!(launch.contains_key(key), "launch lacks `{key}`");
            assert!(!attach.contains_key(key), "attach must not offer `{key}`");
        }
    }

    #[test]
    fn shared_options_are_offered_for_both_requests() {
        let schema = schema();
        for request in ["launch", "attach"] {
            let p = props(variant(&schema, request));
            for key in ["console", "pipeTransport", "sourceFileMap", "logging"] {
                assert!(p.contains_key(key), "`{request}` lacks `{key}`");
            }
        }
    }

    #[test]
    fn every_ref_points_at_an_existing_definition() {
        fn walk(value: &Value, definitions: &Value, refs: &mut usize) {
            match value {
                Value::Object(map) => {
                    for (key, v) in map {
                        if key == "$ref" {
                            let target = v.as_str().expect("$ref is a string");
                            let name = target
                                .strip_prefix("#/definitions/")
                                .unwrap_or_else(|| panic!("unsupported $ref `{target}`"));
                            assert!(definitions.get(name).is_some(), "dangling $ref `{target}`");
                            *refs += 1;
                        } else {
                            walk(v, definitions, refs);
                        }
                    }
                }
                Value::Array(items) => items.iter().for_each(|v| walk(v, definitions, refs)),
                _ => {}
            }
        }
        let schema = schema();
        let mut refs = 0;
        walk(&schema, &schema["definitions"], &mut refs);
        assert!(refs > 0, "the schema is expected to use definitions");
    }

    #[test]
    fn console_only_allows_internal_console() {
        let schema = schema();
        assert_eq!(
            schema["definitions"]["console"]["enum"],
            serde_json::json!(["internalConsole"])
        );
    }

    #[test]
    fn source_file_map_is_documented_as_ignored() {
        let schema = schema();
        let description = schema["definitions"]["sourceFileMap"]["description"]
            .as_str()
            .expect("description");
        assert!(description.contains("IGNORED"), "{description}");
    }
}

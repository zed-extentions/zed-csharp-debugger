//! Pure logic for locating the `netcoredbg` adapter binary and building its
//! launch arguments. Nothing in this module touches the Zed extension API, so
//! it is fully unit-testable on the host target.

/// The debug adapter name registered in `extension.toml` and used in
/// `.zed/debug.json` (`"adapter": "netcoredbg"`) and Zed `dap.*` settings.
pub const ADAPTER_NAME: &str = "netcoredbg";

/// Command line arguments that turn `netcoredbg` into a stdio DAP server
/// speaking the VSCode dialect.
pub const INTERPRETER_ARGS: &[&str] = &["--interpreter=vscode"];

/// Host platform kinds we differentiate binary names for.
///
/// Mirrors `zed_extension_api::Os` without depending on it, to keep this
/// module free of WASM-hosted types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Unix,
}

/// Binary names to probe on `$PATH`, in order of preference.
///
/// `netcoredbg` ships a native `netcoredbg` (Unix) or `netcoredbg.exe`
/// (Windows). We also probe the plain name on Windows because Zed's `which`
/// resolution may or may not apply `PATHEXT` expansion.
pub fn binary_candidates(platform: Platform) -> &'static [&'static str] {
    match platform {
        Platform::Windows => &["netcoredbg", "netcoredbg.exe"],
        Platform::Unix => &["netcoredbg"],
    }
}

/// A resolved adapter invocation (command + arguments).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterCommand {
    pub command: String,
    pub arguments: Vec<String>,
}

/// Configuration for running `netcoredbg` inside a container or over an
/// external pipe program (`docker exec`, `kubectl exec`, `ssh`, etc.).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipeTransport {
    pub pipe_program: String,
    pub pipe_args: Vec<String>,
    pub debugger_path: String,
}

/// Builds the adapter command line for a `pipeTransport` invocation:
/// `command = pipeProgram`,
/// `arguments = pipeArgs + [debuggerPath, "--interpreter=vscode"] + logging_args`.
pub fn resolve_pipe_transport_command(
    pipe: &PipeTransport,
    logging_args: &[String],
) -> Result<AdapterCommand, String> {
    let command = pipe.pipe_program.trim();
    if command.is_empty() {
        return Err("`pipeTransport.pipeProgram` must not be empty".to_string());
    }
    let debugger_path = pipe.debugger_path.trim();
    if debugger_path.is_empty() {
        return Err("`pipeTransport.debuggerPath` must not be empty".to_string());
    }

    let mut arguments =
        Vec::with_capacity(pipe.pipe_args.len() + 1 + INTERPRETER_ARGS.len() + logging_args.len());
    arguments.extend(pipe.pipe_args.iter().cloned());
    arguments.push(debugger_path.to_string());
    arguments.extend(INTERPRETER_ARGS.iter().map(|s| (*s).to_string()));
    arguments.extend(logging_args.iter().cloned());

    Ok(AdapterCommand {
        command: command.to_string(),
        arguments,
    })
}

/// Decides which `netcoredbg` binary to run.
///
/// Priority:
/// 1. `user_provided_path` (Zed setting `dap.netcoredbg.binary`)
/// 2. `which_result` (first hit from `worktree.which()` over
///    [`binary_candidates`])
/// 3. Instructive error telling the user how to install netcoredbg.
pub fn resolve_adapter_command(
    user_provided_path: Option<&str>,
    which_result: Option<&str>,
) -> Result<AdapterCommand, String> {
    // A blank (whitespace-only) path is treated as absent so a stray space in
    // a Zed setting cannot shadow a working PATH lookup.
    let user_path = user_provided_path.map(str::trim).filter(|p| !p.is_empty());
    let which_result = which_result.map(str::trim).filter(|p| !p.is_empty());
    let command = match (user_path, which_result) {
        (Some(user_path), _) => user_path.to_string(),
        (None, Some(found)) => found.to_string(),
        _ => {
            return Err(
                "netcoredbg executable was not found. Install netcoredbg (MIT) and make sure \
                 it is on your PATH, or point Zed to it explicitly:\n\
                 1. Download a release for your platform: https://github.com/Samsung/netcoredbg/releases\n\
                 2. Or build from source: https://github.com/Samsung/netcoredbg\n\
                 3. Then either add its directory to PATH, or set in Zed settings:\n\
                 {\"dap\": {\"netcoredbg\": {\"binary\": \"/path/to/netcoredbg\"}}}"
                    .to_string(),
            )
        }
    };

    Ok(AdapterCommand {
        command,
        arguments: INTERPRETER_ARGS.iter().map(|s| s.to_string()).collect(),
    })
}

/// Resolves the full adapter command line: [`resolve_adapter_command`]
/// (user setting → `PATH` probe), falling back to `download` (FR-16), which
/// returns the path of a downloaded netcoredbg. The diagnostics flags
/// `logging_args` (FR-18) are appended after the interpreter arguments
/// whatever the binary's source, so logging works for user-path, `PATH` and
/// downloaded binaries alike.
///
/// `download` is only called when neither the setting nor `PATH` produced a
/// binary; if it fails, its error is combined with the instructive install
/// error (kept as the offline fallback).
pub fn resolve_adapter_command_with_download(
    user_provided_path: Option<&str>,
    which_result: Option<&str>,
    download: impl FnOnce() -> Result<String, String>,
    logging_args: &[String],
) -> Result<AdapterCommand, String> {
    let mut adapter = match resolve_adapter_command(user_provided_path, which_result) {
        Ok(adapter) => adapter,
        Err(fallback_error) => match download() {
            Ok(command) => AdapterCommand {
                command,
                arguments: INTERPRETER_ARGS.iter().map(|s| s.to_string()).collect(),
            },
            Err(download_error) => {
                return Err(crate::download::combine_download_error(
                    &download_error,
                    &fallback_error,
                ))
            }
        },
    };
    adapter.arguments.extend(logging_args.iter().cloned());
    Ok(adapter)
}

/// Extracts the directory portion of a binary path.
///
/// Handles both `/` and `\` separators so it works on Windows and Unix paths
/// alike (pure string logic; no host filesystem access from WASM).
pub fn parent_directory(path: &str) -> Option<&str> {
    let trimmed = path.trim();
    let idx = trimmed.rfind(['/', '\\'])?;
    if idx == 0 {
        return None;
    }
    Some(&trimmed[..idx])
}

/// Computes a `DOTNET_ROOT` hint for the adapter/debuggee environment.
///
/// netcoredbg needs to locate the .NET runtime when launching a `.dll`
/// program. On Windows in particular it fails to find the runtime unless
/// `DOTNET_ROOT` is set. If the user did not set `DOTNET_ROOT` themselves and
/// we can see a `dotnet` executable, we point `DOTNET_ROOT` at its
/// installation directory (e.g. `C:\Program Files\dotnet\dotnet.exe` ->
/// `C:\Program Files\dotnet`).
///
/// Returns `Some((key, value))` to inject, or `None` when no hint is needed.
pub fn dotnet_root_hint(
    existing_envs: &[(String, String)],
    dotnet_path: Option<&str>,
) -> Option<(String, String)> {
    let has_dotnet_root = existing_envs
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case("DOTNET_ROOT"));
    if has_dotnet_root {
        return None;
    }
    let dotnet_path = dotnet_path?.trim();
    if dotnet_path.is_empty() {
        return None;
    }
    // `which("dotnet")` may return the wrapper that is *inside* the install
    // dir (Unix: /usr/share/dotnet/dotnet -> /usr/share/dotnet) — taking the
    // parent directory of the binary itself is the right answer in both
    // layouts.
    let root = parent_directory(dotnet_path)?;
    if root.is_empty() {
        return None;
    }
    Some(("DOTNET_ROOT".to_string(), root.to_string()))
}

#[cfg(test)]
mod tests {
    use crate::resolve::{
        binary_candidates, dotnet_root_hint, parent_directory, resolve_adapter_command,
        resolve_adapter_command_with_download, resolve_pipe_transport_command, PipeTransport,
        Platform, ADAPTER_NAME, INTERPRETER_ARGS,
    };

    #[test]
    fn adapter_name_is_stable() {
        assert_eq!(ADAPTER_NAME, "netcoredbg");
    }

    #[test]
    fn interpreter_args_enable_vscode_dap_over_stdio() {
        assert_eq!(INTERPRETER_ARGS, &["--interpreter=vscode"]);
    }

    #[test]
    fn binary_candidates_windows_probe_exe_too() {
        assert_eq!(
            binary_candidates(Platform::Windows),
            &["netcoredbg", "netcoredbg.exe"]
        );
    }

    #[test]
    fn binary_candidates_unix_only_plain_name() {
        assert_eq!(binary_candidates(Platform::Unix), &["netcoredbg"]);
    }

    #[test]
    fn user_provided_path_wins_over_which() {
        let resolved =
            resolve_adapter_command(Some("/opt/tools/netcoredbg"), Some("/usr/bin/netcoredbg"))
                .expect("should resolve");
        assert_eq!(resolved.command, "/opt/tools/netcoredbg");
        assert_eq!(resolved.arguments, vec!["--interpreter=vscode"]);
    }

    #[test]
    fn which_result_used_when_no_user_path() {
        let resolved =
            resolve_adapter_command(None, Some("C:\\bin\\netcoredbg.exe")).expect("should resolve");
        assert_eq!(resolved.command, "C:\\bin\\netcoredbg.exe");
        assert_eq!(resolved.arguments, vec!["--interpreter=vscode"]);
    }

    #[test]
    fn blank_user_path_falls_back_to_which() {
        let resolved = resolve_adapter_command(Some("   "), Some("/usr/local/bin/netcoredbg"))
            .expect("should resolve");
        assert_eq!(resolved.command, "/usr/local/bin/netcoredbg");
    }

    #[test]
    fn missing_binary_yields_instructive_error() {
        let err = resolve_adapter_command(None, None).expect_err("should error");
        assert!(err.contains("netcoredbg"), "error should name the binary");
        assert!(
            err.contains("Samsung/netcoredbg"),
            "error should link install instructions, got: {err}"
        );
        assert!(
            err.contains("dap"),
            "error should mention the Zed `dap` setting override"
        );
    }

    #[test]
    fn parent_directory_handles_both_separators() {
        assert_eq!(
            parent_directory("C:\\Program Files\\dotnet\\dotnet.exe"),
            Some("C:\\Program Files\\dotnet")
        );
        assert_eq!(
            parent_directory("/usr/share/dotnet/dotnet"),
            Some("/usr/share/dotnet")
        );
        assert_eq!(parent_directory("dotnet"), None);
        assert_eq!(parent_directory(""), None);
    }

    #[test]
    fn dotnet_root_hint_injected_when_missing() {
        let envs = vec![] as Vec<(String, String)>;
        let hint = dotnet_root_hint(&envs, Some("C:\\Program Files\\dotnet\\dotnet.exe"));
        assert_eq!(
            hint,
            Some((
                "DOTNET_ROOT".to_string(),
                "C:\\Program Files\\dotnet".to_string()
            ))
        );
    }

    #[test]
    fn dotnet_root_hint_skipped_when_user_set_it() {
        let envs = vec![("DOTNET_ROOT".to_string(), "/custom".to_string())];
        assert_eq!(
            dotnet_root_hint(&envs, Some("/usr/share/dotnet/dotnet")),
            None
        );
    }

    #[test]
    fn dotnet_root_hint_case_insensitive_existing_key() {
        let envs = vec![("dotnet_root".to_string(), "X".to_string())];
        assert_eq!(
            dotnet_root_hint(&envs, Some("/usr/share/dotnet/dotnet")),
            None
        );
    }

    #[test]
    fn dotnet_root_hint_skipped_without_dotnet() {
        assert_eq!(dotnet_root_hint(&[], None), None);
        assert_eq!(dotnet_root_hint(&[], Some("   ")), None);
        assert_eq!(dotnet_root_hint(&[], Some("dotnet")), None);
    }

    fn logging() -> Vec<String> {
        vec![
            "--engineLogging".to_string(),
            "--log=/tmp/x.log".to_string(),
        ]
    }

    fn with_logging() -> Vec<String> {
        vec![
            "--interpreter=vscode".to_string(),
            "--engineLogging".to_string(),
            "--log=/tmp/x.log".to_string(),
        ]
    }

    #[test]
    fn logging_args_appended_for_user_path() {
        let adapter = resolve_adapter_command_with_download(
            Some("/opt/netcoredbg/netcoredbg"),
            Some("/usr/bin/netcoredbg"),
            || panic!("download must not run when the user path is set"),
            &logging(),
        )
        .unwrap();
        assert_eq!(adapter.command, "/opt/netcoredbg/netcoredbg");
        assert_eq!(adapter.arguments, with_logging());
    }

    #[test]
    fn logging_args_appended_for_path_probe() {
        let adapter = resolve_adapter_command_with_download(
            None,
            Some("/usr/bin/netcoredbg"),
            || panic!("download must not run when PATH has netcoredbg"),
            &logging(),
        )
        .unwrap();
        assert_eq!(adapter.command, "/usr/bin/netcoredbg");
        assert_eq!(adapter.arguments, with_logging());
    }

    #[test]
    fn logging_args_appended_for_downloaded_binary() {
        let adapter = resolve_adapter_command_with_download(
            None,
            None,
            || Ok("/ext/netcoredbg/3.1.3/netcoredbg/netcoredbg".to_string()),
            &logging(),
        )
        .unwrap();
        assert_eq!(
            adapter.command,
            "/ext/netcoredbg/3.1.3/netcoredbg/netcoredbg"
        );
        assert_eq!(adapter.arguments, with_logging());
    }

    #[test]
    fn no_logging_args_keeps_interpreter_only() {
        let adapter =
            resolve_adapter_command_with_download(None, None, || Ok("nd".to_string()), &[])
                .unwrap();
        assert_eq!(adapter.arguments, vec!["--interpreter=vscode".to_string()]);
    }

    #[test]
    fn download_failure_combines_with_install_error() {
        let error = resolve_adapter_command_with_download(
            None,
            Some("   "),
            || Err("offline".to_string()),
            &logging(),
        )
        .unwrap_err();
        assert!(error.starts_with("Automatic netcoredbg download failed: offline"));
        assert!(error.contains("netcoredbg executable was not found"));
    }

    #[test]
    fn pipe_transport_builds_command_with_interpreter_and_logging() {
        let pipe = PipeTransport {
            pipe_program: " docker ".to_string(),
            pipe_args: vec!["exec".to_string(), "-i".to_string(), "myapp".to_string()],
            debugger_path: " /vsdbg/netcoredbg ".to_string(),
        };
        let adapter = resolve_pipe_transport_command(&pipe, &logging()).unwrap();
        assert_eq!(adapter.command, "docker");
        assert_eq!(
            adapter.arguments,
            vec![
                "exec",
                "-i",
                "myapp",
                "/vsdbg/netcoredbg",
                "--interpreter=vscode",
                "--engineLogging",
                "--log=/tmp/x.log",
            ]
        );
    }

    #[test]
    fn pipe_transport_without_pipe_args_or_logging() {
        let pipe = PipeTransport {
            pipe_program: "netcoredbg-wrapper".to_string(),
            pipe_args: Vec::new(),
            debugger_path: "/opt/netcoredbg".to_string(),
        };
        let adapter = resolve_pipe_transport_command(&pipe, &[]).unwrap();
        assert_eq!(adapter.command, "netcoredbg-wrapper");
        assert_eq!(
            adapter.arguments,
            vec!["/opt/netcoredbg", "--interpreter=vscode"]
        );
    }

    #[test]
    fn pipe_transport_rejects_blank_program_or_debugger_path() {
        let err = resolve_pipe_transport_command(
            &PipeTransport {
                pipe_program: "   ".to_string(),
                pipe_args: Vec::new(),
                debugger_path: "/vsdbg/netcoredbg".to_string(),
            },
            &[],
        )
        .unwrap_err();
        assert!(err.contains("pipeProgram"), "{err}");

        let err = resolve_pipe_transport_command(
            &PipeTransport {
                pipe_program: "docker".to_string(),
                pipe_args: Vec::new(),
                debugger_path: "  ".to_string(),
            },
            &[],
        )
        .unwrap_err();
        assert!(err.contains("debuggerPath"), "{err}");
    }
}

use crate::serde_json::Value;

/// Command line flags netcoredbg accepts on its *adapter* command line
/// (verified against `Samsung/netcoredbg` `src/main.cpp`). These are not DAP
/// launch properties, so they never reach the forwarded configuration.
pub const ENGINE_LOGGING_FLAG: &str = "--engineLogging";

pub const LOG_FLAG: &str = "--log";

/// Keys of the extension's own `logging` dialect.
pub(super) const LOGGING_KEYS: [&str; 4] = ["engine", "enginePath", "log", "logPath"];

/// vscode-csharp `logging` keys (`coreclr` launch/attach schema in
/// dotnet/vscode-csharp `package.json`) that netcoredbg has no equivalent
/// for. They are accepted and ignored so configurations copied from VS Code
/// keep working. `engineLogging` is *not* listed: it maps to
/// `--engineLogging`.
pub const VSCODE_IGNORED_LOGGING_KEYS: [&str; 9] = [
    "exceptions",
    "moduleLoad",
    "programOutput",
    "browserStdOut",
    "elapsedTiming",
    "threadExit",
    "processExit",
    "consoleUsageMessage",
    "diagnosticsLog",
];

/// Normalizes the diagnostics `logging` object of an adapter configuration.
///
/// netcoredbg enables engine/protocol logging through *adapter* command line
/// flags (see [`ENGINE_LOGGING_FLAG`] and [`LOG_FLAG`]), while the user can
/// only customize a debug configuration's JSON. The `logging` object bridges
/// the two: it is translated into adapter arguments and removed from the
/// configuration before it is forwarded to netcoredbg (which would otherwise
/// ignore unknown fields silently, hiding typos).
///
/// Accepted shape (both request kinds):
///
/// ```json
/// { "logging": { "engine": true, "enginePath": "logs/engine.log",
///                "log": true, "logPath": "logs/adapter.log" } }
/// ```
///
/// * `engine` (`bool`, default `false`): pass `--engineLogging`; engine/DAP
///   output lands in the debug console as `console` output events.
/// * `enginePath` (string): pass `--engineLogging=<path>`; implies `engine`
///   and redirects the engine log to the file instead of the console.
/// * `log` (`bool`, default `false`): pass `--log`; netcoredbg's internal
///   protocol log goes to a temp file named after the executable.
/// * `logPath` (string): pass `--log=<path>`; implies `log`. `"stdout"` is
///   rejected: netcoredbg would write its log into the DAP channel.
///
/// vscode-csharp compatibility: `engineLogging: true` (the VS Code spelling)
/// also passes `--engineLogging`; the other vscode-csharp `logging` keys
/// ([`VSCODE_IGNORED_LOGGING_KEYS`]) have no netcoredbg equivalent and are
/// ignored without error.
///
/// Truly unknown keys and wrong types are rejected with actionable errors:
/// the object is consumed here (never forwarded), so a typo such as
/// `"engin": true` would otherwise silently disable the diagnostics the user
/// asked for.
///
/// Relative paths resolve against the adapter's working directory (the
/// worktree root) and the directory must already exist — netcoredbg does not
/// create it and silently drops the log when the file cannot be opened.
///
/// Returns the adapter arguments to append after the interpreter flag. The
/// `logging` key is always removed from `config` when present.
pub fn adapter_logging_args(config: &mut Value) -> Result<Vec<String>, String> {
    let Some(logging) = config.as_object_mut().and_then(|object| {
        object
            .remove("logging")
            .and_then(|value| (!value.is_null()).then_some(value))
    }) else {
        return Ok(Vec::new());
    };

    let logging = logging
        .as_object()
        .ok_or("`logging` must be an object (e.g. {\"logging\": {\"engine\": true}})")?
        .clone();

    let mut engine = false;
    for key in ["engine", "engineLogging"] {
        if let Some(value) = logging.get(key) {
            match value {
                Value::Bool(flag) => engine |= *flag,
                Value::Null => {}
                _ => return Err(format!("`logging.{key}` must be a boolean")),
            }
        }
    }
    let mut log = false;
    if let Some(value) = logging.get("log") {
        match value {
            Value::Bool(flag) => log = *flag,
            Value::Null => {}
            _ => return Err("`logging.log` must be a boolean".to_string()),
        }
    }

    let engine_path = match logging.get("enginePath") {
        None | Some(Value::Null) => None,
        Some(Value::String(path)) => Some(validate_log_path("enginePath", path)?),
        Some(_) => {
            return Err("`logging.enginePath` must be a string file path \
                        (e.g. \"logs/engine.log\")"
                .to_string())
        }
    };
    let log_path = match logging.get("logPath") {
        None | Some(Value::Null) => None,
        Some(Value::String(path)) => Some(validate_log_path("logPath", path)?),
        Some(_) => {
            return Err("`logging.logPath` must be a string file path \
                        (e.g. \"logs/adapter.log\")"
                .to_string())
        }
    };

    if log_path.as_deref() == Some("stdout") {
        return Err(
            "`logging.logPath` must not be \"stdout\": netcoredbg talks DAP over stdout, so \
             its log would corrupt the debug session. Use \"stderr\" or a file path"
                .to_string(),
        );
    }

    for key in logging.keys() {
        let key = key.as_str();
        if !LOGGING_KEYS.contains(&key)
            && key != "engineLogging"
            && !VSCODE_IGNORED_LOGGING_KEYS.contains(&key)
        {
            return Err(format!(
                "`logging.{key}` is not a recognized diagnostics option. \
                 Supported: `engine`, `enginePath`, `log`, `logPath` (and vscode-csharp's \
                 `engineLogging`); ignored vscode-csharp keys: {}",
                VSCODE_IGNORED_LOGGING_KEYS
                    .iter()
                    .map(|key| format!("`{key}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    // A log file path implies the corresponding flag is wanted.
    engine |= engine_path.is_some();
    log |= log_path.is_some();

    let mut arguments = Vec::new();
    if let Some(path) = engine_path {
        arguments.push(format!("{ENGINE_LOGGING_FLAG}={path}"));
    } else if engine {
        arguments.push(ENGINE_LOGGING_FLAG.to_string());
    }
    if let Some(path) = log_path {
        arguments.push(format!("{LOG_FLAG}={path}"));
    } else if log {
        arguments.push(LOG_FLAG.to_string());
    }
    Ok(arguments)
}

/// Validates a log file path coming from the `logging` object.
pub(super) fn validate_log_path(key: &str, path: &str) -> Result<String, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err(format!("`logging.{key}` must not be empty"));
    }
    Ok(path.to_string())
}

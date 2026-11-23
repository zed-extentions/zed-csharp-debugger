use crate::serde_json::{self, Map, Value};

use crate::config::type_name;
/// Normalizes a `launch` configuration in place.
///
/// * validates that `program` is a non-empty string (netcoredbg requires it;
///   it may point at an `app.dll` or a framework-dependent executable)
/// * defaults `cwd` to the worktree root when absent
/// * validates `args` is an array of strings, `env` an object of
///   string -> string, when present
/// * optionally injects a `DOTNET_ROOT` entry into `env` (see
///   [`crate::resolve::dotnet_root_hint`])
pub fn normalize_launch(
    config: &mut Value,
    worktree_root: &str,
    dotnet_path: Option<&str>,
) -> Result<(), String> {
    normalize_console(config)?;

    let root = config
        .as_object_mut()
        .ok_or_else(|| "debug configuration must be a JSON object".to_string())?;

    let program = root
        .get("program")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "launch configuration requires a non-empty `program` field pointing at your \
             application (e.g. \"bin/Debug/net8.0/App.dll\" or a framework-dependent \
             executable)"
                .to_string()
        })?;
    // Re-insert the trimmed program so downstream comparisons are stable.
    root.insert("program".to_string(), Value::String(program.to_string()));

    if let Some(args) = root.get("args") {
        if !args.is_array() || args.as_array().unwrap().iter().any(|a| !a.is_string()) {
            return Err("`args` must be an array of strings".to_string());
        }
    }

    let mut envs: Vec<(String, String)> = Vec::new();
    match root.get("env") {
        None | Some(Value::Null) => {}
        Some(Value::Object(map)) => {
            for (key, value) in map {
                let value = value
                    .as_str()
                    .ok_or_else(|| format!("`env[\"{key}\"]` must be a string value"))?;
                envs.push((key.clone(), value.to_string()));
            }
        }
        Some(_) => {
            return Err("`env` must be an object of string keys to string values".to_string())
        }
    }

    if let Some((key, value)) = crate::resolve::dotnet_root_hint(&envs, dotnet_path) {
        envs.push((key, value.clone()));
        let entry = root
            .entry("env")
            .or_insert_with(|| Value::Object(Map::new()));
        match entry {
            Value::Object(map) => {
                map.insert("DOTNET_ROOT".to_string(), Value::String(value));
            }
            _ => unreachable!("env entry is either absent or an object here"),
        }
    }

    if !root
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
    {
        root.insert("cwd".to_string(), Value::String(worktree_root.to_string()));
    }

    Ok(())
}

/// Normalizes an `attach` configuration in place.
///
/// netcoredbg accepts `processId` as a number or a numeric string; we accept
/// both but coerce numeric strings to integers so the value we forward always
/// matches the DAP spec.
pub fn normalize_attach(config: &mut Value) -> Result<(), String> {
    normalize_console(config)?;

    let root = config
        .as_object_mut()
        .ok_or_else(|| "debug configuration must be a JSON object".to_string())?;

    match root.get("processId") {
        Some(Value::Number(number)) => {
            let id = number
                .as_u64()
                .ok_or_else(|| "`processId` must be a positive integer".to_string())?;
            root.insert(
                "processId".to_string(),
                Value::Number(serde_json::Number::from(id)),
            );
        }
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            // Preserve Zed's native process-picker macro untouched so Zed can substitute the picked PID.
            if trimmed == "${ZED_PICK_PID}" || trimmed == "$ZED_PICK_PID" {
                root.insert("processId".to_string(), Value::String(trimmed.to_string()));
            } else {
                let id = trimmed
                    .parse::<u64>()
                    .map_err(|_| format!("`processId` must be an integer, got string {raw:?}"))?;
                root.insert(
                    "processId".to_string(),
                    Value::Number(serde_json::Number::from(id)),
                );
            }
        }
        Some(other) => {
            return Err(format!(
                "`processId` must be an integer or numeric string, got {}",
                type_name(other)
            ))
        }
        None => {
            return Err(
                "attach configuration requires a `processId` field with the PID of the \
                 process to attach to"
                    .to_string(),
            )
        }
    };

    Ok(())
}

/// Validates and strips the optional `console` property.
///
/// netcoredbg never sends DAP `runInTerminal` (`src/protocols/vscodeprotocol.cpp`),
/// so only `"internalConsole"` is supported. Terminal modes fail with an
/// actionable attach workaround.
pub fn normalize_console(config: &mut Value) -> Result<(), String> {
    let Some(root) = config.as_object_mut() else {
        return Err("debug configuration must be a JSON object".to_string());
    };
    let Some(raw) = root.remove("console") else {
        return Ok(());
    };
    match raw {
        Value::Null => Ok(()),
        Value::String(mode) => {
            let mode = mode.trim();
            match mode {
                "internalConsole" => Ok(()),
                "integratedTerminal" | "externalTerminal" => Err(format!(
                    "unsupported `console` mode {mode:?}: netcoredbg does not send DAP \
                     `runInTerminal`, so debuggee stdin/TTY is unavailable. Run your app \
                     from a Zed task or terminal instead, and attach using \
                     `{{\"request\": \"attach\", \"processId\": \"${{ZED_PICK_PID}}\"}}`"
                )),
                other => Err(format!(
                    "invalid `console` value {other:?}: only \"internalConsole\" is \
                     supported (netcoredbg lacks DAP `runInTerminal`; for interactive \
                     console apps, run from a Zed task/terminal and attach with \
                     `\"processId\": \"${{ZED_PICK_PID}}\"`)"
                )),
            }
        }
        other => Err(format!(
            "`console` must be \"internalConsole\", got {}",
            type_name(&other)
        )),
    }
}

/// Extracts and strips the optional `pipeTransport` object for container or
/// remote adapter invocation.
pub fn adapter_pipe_transport(
    config: &mut Value,
) -> Result<Option<crate::resolve::PipeTransport>, String> {
    let Some(raw) = config
        .as_object_mut()
        .and_then(|object| object.remove("pipeTransport"))
        .filter(|value| !value.is_null())
    else {
        return Ok(None);
    };

    let map = raw.as_object().ok_or_else(|| {
        "`pipeTransport` must be an object with `pipeProgram`, optional `pipeArgs`, \
         and `debuggerPath`"
            .to_string()
    })?;

    for key in map.keys() {
        if !matches!(key.as_str(), "pipeProgram" | "pipeArgs" | "debuggerPath") {
            return Err(format!(
                "`pipeTransport.{key}` is not a supported option. Supported: \
                 `pipeProgram`, `pipeArgs`, `debuggerPath`"
            ));
        }
    }

    let pipe_program = match map.get("pipeProgram") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        Some(Value::String(_)) | None | Some(Value::Null) => {
            return Err(
                "`pipeTransport.pipeProgram` is required and must be a non-empty string \
                 (e.g. \"docker\")"
                    .to_string(),
            )
        }
        Some(other) => {
            return Err(format!(
                "`pipeTransport.pipeProgram` must be a string, got {}",
                type_name(other)
            ))
        }
    };

    let debugger_path = match map.get("debuggerPath") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        Some(Value::String(_)) | None | Some(Value::Null) => {
            return Err(
                "`pipeTransport.debuggerPath` is required and must be a non-empty string \
                 (e.g. \"/usr/local/bin/netcoredbg\")"
                    .to_string(),
            )
        }
        Some(other) => {
            return Err(format!(
                "`pipeTransport.debuggerPath` must be a string, got {}",
                type_name(other)
            ))
        }
    };

    let pipe_args = match map.get("pipeArgs") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => {
            let mut args = Vec::with_capacity(items.len());
            for item in items {
                let s = item.as_str().ok_or_else(|| {
                    "`pipeTransport.pipeArgs` must be an array of strings".to_string()
                })?;
                args.push(s.to_string());
            }
            args
        }
        Some(_) => return Err("`pipeTransport.pipeArgs` must be an array of strings".to_string()),
    };

    Ok(Some(crate::resolve::PipeTransport {
        pipe_program,
        pipe_args,
        debugger_path,
    }))
}

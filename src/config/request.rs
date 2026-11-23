use crate::serde_json::{self, Map, Value};

/// Whether a debug configuration launches a new debuggee or attaches to an
/// existing process. Mirrors `StartDebuggingRequestArgumentsRequest` without
/// importing the WASM-hosted type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    Launch,
    Attach,
}

/// Determines the request kind from the user's adapter configuration JSON.
pub fn request_kind(config: &Value) -> Result<RequestKind, String> {
    match config.get("request").and_then(Value::as_str) {
        Some("launch") => Ok(RequestKind::Launch),
        Some("attach") => Ok(RequestKind::Attach),
        Some(other) => Err(format!(
            "invalid `request` value: {other:?}. Expected \"launch\" or \"attach\""
        )),
        None => Err(
            "debug configuration is missing the required `request` field \
             (\"launch\" or \"attach\")"
                .to_string(),
        ),
    }
}

/// A debugger-agnostic debug request coming from Zed's new-process modal or
/// from a locator's second phase. Mirrors `zed_extension_api::DebugRequest`
/// (`dap.wit` `debug-request`) without importing the WASM-hosted type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenericRequest {
    Launch {
        program: String,
        cwd: Option<String>,
        args: Vec<String>,
        envs: Vec<(String, String)>,
    },
    Attach {
        process_id: Option<u32>,
    },
}

/// Converts a debugger-agnostic request (`dap.wit` `debug-config`) into the
/// netcoredbg adapter configuration that becomes the scenario's `config`.
///
/// * launch: `program`, plus `args` (when non-empty), `cwd` (when present and
///   non-blank; otherwise [`normalize_launch`] defaults it to the worktree
///   root), `env` (when non-empty)
/// * attach: `processId` only when Zed supplied one. Emitting
///   `"processId": null` would make [`normalize_attach`] fail with a
///   confusing type error; omitting it yields the actionable "requires a
///   `processId`" error instead.
/// * `stop_on_entry` maps to netcoredbg's `stopAtEntry` for launch requests.
///   netcoredbg's `attach` handler reads only `processId`, so it is not
///   emitted for attach.
pub fn scenario_config(request: &GenericRequest, stop_on_entry: Option<bool>) -> Value {
    let mut config = Map::new();
    match request {
        GenericRequest::Launch {
            program,
            cwd,
            args,
            envs,
        } => {
            config.insert("request".to_string(), Value::String("launch".to_string()));
            config.insert("program".to_string(), Value::String(program.clone()));
            if !args.is_empty() {
                config.insert(
                    "args".to_string(),
                    Value::Array(args.iter().cloned().map(Value::String).collect()),
                );
            }
            if let Some(cwd) = cwd.as_deref().filter(|cwd| !cwd.trim().is_empty()) {
                config.insert("cwd".to_string(), Value::String(cwd.to_string()));
            }
            if !envs.is_empty() {
                let env = envs
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                    .collect::<Map<String, Value>>();
                config.insert("env".to_string(), Value::Object(env));
            }
            if let Some(stop_on_entry) = stop_on_entry {
                config.insert("stopAtEntry".to_string(), Value::Bool(stop_on_entry));
            }
        }
        GenericRequest::Attach { process_id } => {
            config.insert("request".to_string(), Value::String("attach".to_string()));
            if let Some(process_id) = process_id {
                config.insert(
                    "processId".to_string(),
                    Value::Number(serde_json::Number::from(*process_id)),
                );
            }
        }
    }
    Value::Object(config)
}

//! Pure parsing/normalization of the user's `.zed/debug.json` adapter
//! configuration (the JSON blob inside a `DebugTaskDefinition`).
//!
//! The normalized JSON is forwarded to netcoredbg verbatim as the DAP
//! `launch`/`attach` request arguments, so unknown fields are preserved for
//! forward compatibility with netcoredbg releases that understand more of the
//! vscode-csharp `launch.json` dialect than we model here.
//!
//! Supported fields (verified against netcoredbg's
//! `src/protocols/vscodeprotocol.cpp` and `src/main.cpp`):
//!
//! * launch: `program` (required), `args`, `cwd`, `env`, `stopAtEntry`,
//!   `justMyCode`, `enableStepFiltering`
//! * attach: `processId` (required; integer or numeric string)
//! * extension-consumed (stripped before forwarding to netcoredbg):
//!   * `logging` (both: converted into adapter CLI flags `--engineLogging` / `--log`)
//!   * `pipeTransport` (both: launches `netcoredbg` via `pipeProgram` + `pipeArgs`)
//!   * `console` (both: accepts `"internalConsole"`, rejects terminal modes lacking `runInTerminal`)
//!   * `launchSettingsProfile` / `launchSettingsFilePath` (launch: merges `launchSettings.json` profile)
//!   * `envFile` (launch: loads dotenv `KEY=VALUE` pairs)

use crate::serde_json::Value;

mod env_file;
mod jsonc;
mod launch;
mod launch_json;
mod launch_settings;
mod logging;
mod paths;
mod request;

pub use self::env_file::{parse_env_file, take_env_file};
pub use self::launch::{adapter_pipe_transport, normalize_attach, normalize_launch};
pub use self::launch_json::translate_launch_json;
pub use self::launch_settings::{
    merge_launch_sources, parse_launch_settings_profile, take_launch_settings_spec,
};
pub use self::logging::adapter_logging_args;
pub use self::paths::normalize_worktree_relative_path;
pub use self::request::{request_kind, scenario_config, GenericRequest, RequestKind};

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests;

//! Pure logic for the `dotnet build` debug locator (FR-17): task matching,
//! `dotnet build` argument parsing, `dotnet msbuild -getProperty` query
//! construction and output parsing. Nothing in this module touches the Zed
//! extension API, so it is fully unit-testable on the host target.
//!
//! Two-phase locator flow (verified against Zed's host implementation,
//! `crates/debugger_ui/src/session/running.rs`):
//!
//! 1. `dap_locator_create_scenario` turns a user's `dotnet build` task into
//!    a debug scenario carrying that build task.
//! 2. Zed runs the build; on success it hands the *resolved* task back to
//!    `run_dap_locator`, which locates the built artifact. The returned
//!    `DebugRequest::Launch` is finally converted by `dap_config_to_scenario`
//!    (FR-19) into the adapter configuration.
//!
//! Artifact resolution asks MSBuild itself instead of guessing:
//! `dotnet msbuild <project> -getProperty:TargetPath;...` with the task's
//! output-affecting switches forwarded as global properties the way the
//! `dotnet` CLI forwards them (dotnet/sdk
//! `test/dotnet.Tests/CommandTests/MSBuild/GivenDotnetBuildInvocation.cs`).
//! With more than one property MSBuild prints a `{"Properties": {...}}` JSON
//! document; it only evaluates the project, nothing is built (verified on
//! .NET SDK 9).

mod build_args;
mod command;
mod msbuild;
mod paths;

pub use self::build_args::parse_dotnet_build_args;
pub use self::command::{dotnet_build_args, prepare_test_task_for_build, split_command_line};
pub use self::msbuild::{
    msbuild_failure_message, msbuild_query_args, parse_msbuild_properties, resolve_program,
};
pub use self::paths::is_absolute_path;

/// The locator id registered under `[debug_locators]` in `extension.toml`.
pub const LOCATOR_NAME: &str = "dotnet-build";

/// MSBuild properties queried to locate the build output. Requesting more
/// than one property makes MSBuild print JSON (a single property would be
/// printed as a bare value).
pub const MSBUILD_PROPERTIES: &str =
    "-getProperty:TargetPath;TargetFramework;TargetFrameworks;MSBuildProjectFullPath";

/// Project file extensions accepted as an explicit build target.
const PROJECT_EXTENSIONS: [&str; 4] = [".csproj", ".fsproj", ".vbproj", ".proj"];

/// Solution file extensions. MSBuild cannot evaluate properties of a
/// solution (error MSB1063), so these are rejected with a remedy.
const SOLUTION_EXTENSIONS: [&str; 3] = [".sln", ".slnx", ".slnf"];

/// Builds the empty placeholder configuration for a scenario produced by
/// [`crate::locator`] phase 1.
///
/// Zed (`crates/debugger_ui/src/session/running.rs`) calls `dap_request_kind`
/// before running the build task and only invokes phase 2 (`run_dap_locator` +
/// `dap_config_to_scenario`) when `request_kind` returns `Err` (`!config_is_valid`).
/// Omitting `"request"` mirrors Zed's Cargo locator so phase 2 resolves `program`
/// and produces the final `launch` configuration after `dotnet build` succeeds.
pub fn scenario_config() -> String {
    "{}".to_string()
}

#[cfg(test)]
mod tests;

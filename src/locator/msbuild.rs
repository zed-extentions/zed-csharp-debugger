use crate::serde_json::{self, Value};

use crate::locator::build_args::{BuildTarget, ParsedDotnetBuild};
use crate::locator::paths::{is_absolute_path, join_path, normalize_separators, parent_directory};
use crate::locator::MSBUILD_PROPERTIES;

pub(super) fn solution_error(solution: &str) -> String {
    format!(
        "the `dotnet build` task builds a solution ({solution}), and MSBuild cannot report \
         the output of a solution (MSB1063). Point the task at the project you want to \
         debug, e.g. `dotnet build src/App/App.csproj`, or set an explicit `program` in the \
         debug configuration"
    )
}

/// Builds the `dotnet msbuild <target> -getProperty:...` argument list for a
/// parsed build task, forwarding output-affecting switches as global
/// properties like the `dotnet` CLI does. Relative paths are resolved
/// against the task cwd, as the CLI resolves them against its own working
/// directory.
///
/// `task_cwd` must be absolute (the locator receives the *resolved* task,
/// whose cwd Zed has already substituted): the extension cannot set a
/// working directory for the query process, so every path is absolute.
pub fn msbuild_query_args(
    parsed: &ParsedDotnetBuild,
    task_cwd: &str,
) -> Result<Vec<String>, String> {
    let task_cwd = normalize_separators(task_cwd.trim());
    if task_cwd.is_empty() || !is_absolute_path(&task_cwd) {
        return Err(
            "the debug task has no absolute `cwd`, so the `dotnet build` output cannot be \
             located; add `\"cwd\": \"$ZED_WORKTREE_ROOT\"` to the task or set an explicit \
             `program` in the debug configuration"
                .to_string(),
        );
    }

    let target = match &parsed.target {
        Some(BuildTarget::Solution(solution)) => return Err(solution_error(solution)),
        Some(BuildTarget::Project(path) | BuildTarget::Directory(path)) => {
            join_path(&task_cwd, path)
        }
        None => task_cwd.clone(),
    };

    let mut args = vec![
        "msbuild".to_string(),
        target,
        MSBUILD_PROPERTIES.to_string(),
    ];
    if let Some(configuration) = &parsed.configuration {
        args.push(format!("--property:Configuration={configuration}"));
    }
    if let Some(framework) = &parsed.framework {
        args.push(format!("--property:TargetFramework={framework}"));
    }
    if let Some(runtime) = &parsed.runtime {
        args.push(format!("--property:RuntimeIdentifier={runtime}"));
        args.push("--property:_CommandLineDefinedRuntimeIdentifier=true".to_string());
    }
    if parsed.use_current_runtime {
        args.push("--property:UseCurrentRuntimeIdentifier=True".to_string());
    }
    if let Some(output) = &parsed.output {
        let mut output = join_path(&task_cwd, output);
        if !output.ends_with('/') {
            output.push('/');
        }
        args.push(format!("--property:OutputPath={output}"));
        args.push("--property:_CommandLineDefinedOutputPath=true".to_string());
    }
    if let Some(artifacts_path) = &parsed.artifacts_path {
        args.push(format!(
            "--property:ArtifactsPath={}",
            join_path(&task_cwd, artifacts_path)
        ));
    }
    for property in &parsed.properties {
        args.push(format!("--property:{property}"));
    }
    Ok(args)
}

/// Turns a failed `dotnet msbuild` query into an actionable error. MSBuild
/// writes its errors to stdout and localizes the text, so detection keys on
/// the stable `MSBxxxx` codes.
pub fn msbuild_failure_message(status: Option<i32>, stdout: &str, stderr: &str) -> String {
    let output = format!("{}\n{}", stdout.trim(), stderr.trim());
    let output = output.trim();
    if output.contains("MSB1063") {
        return solution_error("the task's directory contains only a solution file");
    }
    let hint = if output.contains("MSB1011") || output.contains("MSB1050") {
        " The task's directory contains more than one project or solution file; name the \
         project to debug in the task args (e.g. `dotnet build src/App/App.csproj`)."
    } else if output.contains("MSB1003") || output.contains("MSB1009") {
        " No project file was found; point the task's `cwd` or args at the project to debug."
    } else if output.contains("MSB1001") {
        " `-getProperty` requires .NET SDK 8 or newer."
    } else {
        ""
    };
    format!(
        "`dotnet msbuild` failed while locating the build output (exit status {status:?}).\
         {hint} Output: {output:.600}"
    )
}

/// MSBuild properties relevant for locating the build output.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MsbuildProperties {
    pub target_path: Option<String>,
    pub target_framework: Option<String>,
    pub target_frameworks: Option<String>,
    pub project_full_path: Option<String>,
}

/// Parses the JSON document printed by `dotnet msbuild` for
/// [`MSBUILD_PROPERTIES`]. Empty strings (e.g. `TargetPath` of a
/// multi-target project's outer build) are treated as absent.
pub fn parse_msbuild_properties(stdout: &str) -> Result<MsbuildProperties, String> {
    let value: Value = serde_json::from_str(stdout.trim()).map_err(|_| {
        format!(
            "`dotnet msbuild -getProperty` did not return the build properties (it requires \
             .NET SDK 8+ and a resolvable project). Output: {:.400}",
            stdout.trim()
        )
    })?;

    let properties = value.get("Properties").ok_or_else(|| {
        format!("`dotnet msbuild -getProperty` output has no `Properties` object: {stdout:.400}")
    })?;

    fn read(parent: &Value, key: &str) -> Option<String> {
        parent
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    Ok(MsbuildProperties {
        target_path: read(properties, "TargetPath"),
        target_framework: read(properties, "TargetFramework"),
        target_frameworks: read(properties, "TargetFrameworks"),
        project_full_path: read(properties, "MSBuildProjectFullPath"),
    })
}

/// The program to launch and its working directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedProgram {
    /// MSBuild's `TargetPath` (absolute path of the built assembly).
    pub program: String,
    /// The project directory (what `dotnet run` uses), falling back to the
    /// task cwd.
    pub cwd: String,
}

/// Resolves the program to launch from the queried MSBuild properties.
///
/// A multi-target project (`<TargetFrameworks>`) queried without `-f` has
/// no `TargetPath`; the error tells the user how to disambiguate.
pub fn resolve_program(
    properties: &MsbuildProperties,
    task_cwd: &str,
) -> Result<ResolvedProgram, String> {
    let program = properties.target_path.clone().ok_or_else(|| {
        if properties.target_framework.is_none() && properties.target_frameworks.is_some() {
            format!(
                "the project targets several frameworks ({}), so the build output is \
                 ambiguous. Add `-f <tfm>` (or `--framework <tfm>`) to the `dotnet build` \
                 task args, or set an explicit `program` in the debug configuration",
                properties.target_frameworks.as_deref().unwrap_or_default()
            )
        } else {
            "MSBuild returned no `TargetPath` for the build target; set an explicit `program` \
             in the debug configuration"
                .to_string()
        }
    })?;

    let cwd = properties
        .project_full_path
        .as_deref()
        .and_then(parent_directory)
        .unwrap_or_else(|| normalize_separators(task_cwd.trim()));

    Ok(ResolvedProgram { program, cwd })
}

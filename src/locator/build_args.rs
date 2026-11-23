use crate::locator::{LOCATOR_NAME, PROJECT_EXTENSIONS, SOLUTION_EXTENSIONS};

/// The build target named on the `dotnet build` command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildTarget {
    /// A project file (`*.csproj`, `*.fsproj`, `*.vbproj`, `*.proj`).
    Project(String),
    /// A solution file (`*.sln`, `*.slnx`, `*.slnf`).
    Solution(String),
    /// Anything else: a directory MSBuild searches for a single project.
    Directory(String),
}

/// The output-affecting bits parsed out of a `dotnet build` argument list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedDotnetBuild {
    pub target: Option<BuildTarget>,
    /// `-c/--configuration`
    pub configuration: Option<String>,
    /// `-f/--framework`
    pub framework: Option<String>,
    /// `-r/--runtime`
    pub runtime: Option<String>,
    /// `--ucr/--use-current-runtime`
    pub use_current_runtime: bool,
    /// `-o/--output`, as written (relative to the task cwd)
    pub output: Option<String>,
    /// `--artifacts-path`, as written (relative to the task cwd)
    pub artifacts_path: Option<String>,
    /// `-p/--property/-property//p` values (`Name=Value[;Name=Value]`)
    pub properties: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ValueOption {
    Configuration,
    Framework,
    Runtime,
    Output,
    ArtifactsPath,
    Property,
    Arch,
    Os,
    /// Value-taking switches that do not affect the output path.
    Ignored,
    /// `dotnet test`-only value switches (excluded when rewriting `test` -> `build`
    /// and ignored when locating `TargetPath`).
    TestOnly,
}

/// `dotnet build` / `dotnet test` switches that take a value (`dotnet build --help`,
/// `dotnet test --help`, .NET SDK 9), plus the MSBuild-style property spellings the CLI accepts.
pub(super) const VALUE_OPTIONS: [(&str, ValueOption); 42] = [
    ("-c", ValueOption::Configuration),
    ("--configuration", ValueOption::Configuration),
    ("-f", ValueOption::Framework),
    ("--framework", ValueOption::Framework),
    ("-r", ValueOption::Runtime),
    ("--runtime", ValueOption::Runtime),
    ("-o", ValueOption::Output),
    ("--output", ValueOption::Output),
    ("--artifacts-path", ValueOption::ArtifactsPath),
    ("-p", ValueOption::Property),
    ("--property", ValueOption::Property),
    ("-property", ValueOption::Property),
    ("/p", ValueOption::Property),
    ("/property", ValueOption::Property),
    ("-a", ValueOption::Arch),
    ("--arch", ValueOption::Arch),
    ("--os", ValueOption::Os),
    ("-v", ValueOption::Ignored),
    ("--verbosity", ValueOption::Ignored),
    ("--version-suffix", ValueOption::Ignored),
    ("-s", ValueOption::TestOnly),
    ("--settings", ValueOption::TestOnly),
    ("-e", ValueOption::TestOnly),
    ("--environment", ValueOption::TestOnly),
    ("--filter", ValueOption::TestOnly),
    ("--test-adapter-path", ValueOption::TestOnly),
    ("-l", ValueOption::TestOnly),
    ("--logger", ValueOption::TestOnly),
    ("-d", ValueOption::TestOnly),
    ("--diag", ValueOption::TestOnly),
    ("--results-directory", ValueOption::TestOnly),
    ("--collect", ValueOption::TestOnly),
    ("--blame-crash-dump-type", ValueOption::TestOnly),
    ("--blame-hang-dump-type", ValueOption::TestOnly),
    ("--blame-hang-timeout", ValueOption::TestOnly),
    // Microsoft.Testing.Platform (MTP) options in case passed before `--`:
    ("--filter-method", ValueOption::TestOnly),
    ("--filter-class", ValueOption::TestOnly),
    ("--filter-namespace", ValueOption::TestOnly),
    ("--filter-display-name", ValueOption::TestOnly),
    ("--filter-uid", ValueOption::TestOnly),
    ("--diagnostic-output-directory", ValueOption::TestOnly),
    ("--report-trx-filename", ValueOption::TestOnly),
];

/// Boolean/flag-only `dotnet test` switches that have no `dotnet build` equivalent.
pub(super) const TEST_ONLY_FLAGS: [&str; 8] = [
    "-t",
    "--list-tests",
    "--no-build",
    "--blame",
    "--blame-crash",
    "--blame-crash-collect-always",
    "--blame-hang",
    "--report-trx",
];

/// Switches that take an optional boolean value (`--sc false`).
pub(super) const OPTIONAL_BOOL_OPTIONS: [&str; 3] = ["--sc", "--self-contained", "--interactive"];

/// Matches `token` against a value-taking switch: `-c Release` (value in the
/// next token), `-c:Release` or `--configuration=Release` (inline value).
/// Returns the option and its inline value, if any.
pub(super) fn match_value_option(token: &str) -> Option<(ValueOption, Option<&str>)> {
    VALUE_OPTIONS.iter().find_map(|(name, option)| {
        if token == *name {
            return Some((*option, None));
        }
        let rest = token.strip_prefix(name)?;
        rest.strip_prefix(':')
            .or_else(|| rest.strip_prefix('='))
            .map(|value| (*option, Some(value)))
    })
}

pub(super) fn classify_target(token: &str) -> BuildTarget {
    let lower = token.to_ascii_lowercase();
    if PROJECT_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
        BuildTarget::Project(token.to_string())
    } else if SOLUTION_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
        BuildTarget::Solution(token.to_string())
    } else {
        BuildTarget::Directory(token.to_string())
    }
}

/// Parses the arguments that follow `dotnet build`.
///
/// Errors on switches whose effect on the output path the locator cannot
/// reproduce (`-a/--arch`, `--os`: the CLI derives the runtime identifier
/// from the host SDK), on a switch missing its value, and on more than one
/// build target. Other unknown switches are ignored.
pub fn parse_dotnet_build_args(args: &[String]) -> Result<ParsedDotnetBuild, String> {
    let mut parsed = ParsedDotnetBuild::default();
    let mut index = 0;
    while let Some(token) = args.get(index) {
        let token = token.as_str();
        index += 1;

        if token == "--" {
            // Everything after `--` is forwarded to the test runner (e.g. MTP), not MSBuild.
            break;
        }
        if let Some(project) = token
            .strip_prefix("--project=")
            .or_else(|| token.strip_prefix("--project:"))
        {
            let project = project.trim();
            if project.is_empty() {
                return Err("`dotnet test` switch `--project` has an empty value".to_string());
            }
            if let Some(existing) = &parsed.target {
                return Err(format!(
                    "the `dotnet build` task names more than one build target ({existing:?} and \
                     {project:?}); point it at the single project to debug"
                ));
            }
            parsed.target = Some(classify_target(project));
            continue;
        }

        if let Some((option, inline)) = match_value_option(token) {
            let value = match inline {
                Some(value) => value,
                None => {
                    let value = args.get(index).ok_or_else(|| {
                        format!("`dotnet build` switch `{token}` is missing its value")
                    })?;
                    index += 1;
                    value.as_str()
                }
            };
            let value = value.trim().to_string();
            if value.is_empty() {
                return Err(format!(
                    "`dotnet build` switch `{token}` has an empty value"
                ));
            }
            match option {
                ValueOption::Configuration => parsed.configuration = Some(value),
                ValueOption::Framework => parsed.framework = Some(value),
                ValueOption::Runtime => parsed.runtime = Some(value),
                ValueOption::Output => parsed.output = Some(value),
                ValueOption::ArtifactsPath => parsed.artifacts_path = Some(value),
                ValueOption::Property => parsed.properties.push(value),
                ValueOption::Arch | ValueOption::Os => {
                    return Err(format!(
                        "the `{LOCATOR_NAME}` locator cannot resolve the build output of \
                         `dotnet build {token}`: the runtime identifier it implies depends on \
                         the host SDK. Use `-r <runtime-identifier>` (e.g. `-r linux-x64`) in \
                         the task instead, or set an explicit `program` in the debug \
                         configuration"
                    ))
                }
                ValueOption::Ignored | ValueOption::TestOnly => {}
            }
            continue;
        }

        if token == "--ucr" || token == "--use-current-runtime" {
            parsed.use_current_runtime = true;
            continue;
        }
        if OPTIONAL_BOOL_OPTIONS.contains(&token) {
            if matches!(args.get(index).map(String::as_str), Some("true" | "false")) {
                index += 1;
            }
            continue;
        }
        if token.starts_with('-') {
            // Flags (`--no-restore`, `--nologo`, ...) and MSBuild
            // pass-through switches (`-bl`, `-tl:off`, ...).
            continue;
        }

        if let Some(existing) = &parsed.target {
            return Err(format!(
                "the `dotnet build` task names more than one build target ({existing:?} and \
                 {token:?}); point it at the single project to debug"
            ));
        }
        parsed.target = Some(classify_target(token));
    }
    Ok(parsed)
}

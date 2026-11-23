use crate::locator::build_args::{
    match_value_option, ValueOption, OPTIONAL_BOOL_OPTIONS, TEST_ONLY_FLAGS,
};

/// Splits a task `command` string into tokens on whitespace, honoring
/// double and single quotes (so `"C:\Program Files\dotnet\dotnet.exe" build`
/// keeps the executable as one token).
pub fn split_command_line(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut quote: Option<char> = None;
    for ch in command.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => current.push(ch),
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                in_token = true;
            }
            None if ch.is_whitespace() => {
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            None => {
                current.push(ch);
                in_token = true;
            }
        }
    }
    if in_token {
        tokens.push(current);
    }
    tokens
}

/// Whether a token names the `dotnet` host: `dotnet` or `dotnet.exe`
/// (case-insensitive), optionally with a directory prefix.
pub fn is_dotnet_executable(token: &str) -> bool {
    let name = token.rsplit(['/', '\\']).next().unwrap_or(token);
    let name = name.to_ascii_lowercase();
    name == "dotnet" || name == "dotnet.exe"
}

/// The `dotnet` subcommand matched by the locator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DotnetSubcommand {
    Build,
    Test,
}

/// Parses a Zed task's `command` and `args` as a `dotnet build` or
/// `dotnet test` invocation, returning `(subcommand, dotnet_executable, rest_args)`.
pub fn dotnet_subcommand_args(
    command: &str,
    args: &[String],
) -> Option<(DotnetSubcommand, String, Vec<String>)> {
    let mut tokens = split_command_line(command);
    tokens.extend(args.iter().cloned());
    let mut tokens = tokens.into_iter();
    let executable = tokens.next()?;
    if !is_dotnet_executable(&executable) {
        return None;
    }
    let subcommand = match tokens.next()?.as_str() {
        "build" => DotnetSubcommand::Build,
        "test" => DotnetSubcommand::Test,
        _ => return None,
    };
    Some((subcommand, executable, tokens.collect()))
}

/// Returns the arguments following `build` or `test` when the task is a
/// `dotnet build` or `dotnet test` invocation, `None` otherwise.
///
/// Accepts `"command": "dotnet", "args": ["build"|"test", ...]`, the
/// single-string form `"command": "dotnet build -c Release"`, and
/// `dotnet.exe` / a path to the `dotnet` host.
pub fn dotnet_build_args(command: &str, args: &[String]) -> Option<Vec<String>> {
    dotnet_subcommand_args(command, args).map(|(_, _, rest)| rest)
}

/// Converts a `dotnet test` task into an equivalent `dotnet build` command
/// and argument list for the locator's pre-launch build step, stripping
/// `dotnet test`-only switches and any `-- <runner args>` tail so failing
/// tests or test flags do not abort or break the build phase.
///
/// Returns `None` if the task is already `dotnet build` or does not match.
pub fn prepare_test_task_for_build(
    command: &str,
    args: &[String],
) -> Option<(String, Vec<String>)> {
    let (subcommand, executable, rest) = dotnet_subcommand_args(command, args)?;
    if subcommand != DotnetSubcommand::Test {
        return None;
    }

    let mut build_args = Vec::with_capacity(rest.len() + 1);
    build_args.push("build".to_string());

    let mut index = 0;
    while let Some(token) = rest.get(index) {
        let token_str = token.as_str();
        index += 1;

        if token_str == "--" {
            // Everything after `--` belongs to the test runner (e.g. MTP `--filter-method`).
            break;
        }
        if let Some(project) = token_str
            .strip_prefix("--project=")
            .or_else(|| token_str.strip_prefix("--project:"))
        {
            if !project.trim().is_empty() {
                build_args.push(project.to_string());
            }
            continue;
        }
        if token_str == "--project" {
            // Convert `--project <path>` to positional `<path>` for `dotnet build`.
            continue;
        }

        if let Some((option, inline)) = match_value_option(token_str) {
            if option == ValueOption::TestOnly {
                if inline.is_none() && index < rest.len() {
                    index += 1;
                }
                continue;
            }
            build_args.push(token.clone());
            if inline.is_none() {
                if let Some(val) = rest.get(index) {
                    build_args.push(val.clone());
                    index += 1;
                }
            }
            continue;
        }

        if TEST_ONLY_FLAGS.contains(&token_str) {
            continue;
        }

        if OPTIONAL_BOOL_OPTIONS.contains(&token_str) {
            build_args.push(token.clone());
            if matches!(rest.get(index).map(String::as_str), Some("true" | "false")) {
                build_args.push(rest[index].clone());
                index += 1;
            }
            continue;
        }

        build_args.push(token.clone());
    }

    Some((executable, build_args))
}

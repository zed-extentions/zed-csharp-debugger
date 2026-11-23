use crate::serde_json::{Map, Value};

use crate::config::jsonc::parse_jsonc;
use crate::config::type_name;
/// Default relative path for ASP.NET Core / .NET `launchSettings.json`.
pub const DEFAULT_LAUNCH_SETTINGS_PATH: &str = "Properties/launchSettings.json";

/// Requested `launchSettings.json` file path and optional profile name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchSettingsSpec {
    pub file_path: String,
    pub profile: Option<String>,
}

/// Extracted fields from a `launchSettings.json` profile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchSettingsProfile {
    pub environment_variables: Vec<(String, String)>,
    pub application_url: Option<String>,
    pub command_line_args: Option<Vec<String>>,
    pub working_directory: Option<String>,
}

/// Extracts and strips `launchSettingsProfile` and `launchSettingsFilePath`
/// from a debug configuration.
pub fn take_launch_settings_spec(config: &mut Value) -> Result<Option<LaunchSettingsSpec>, String> {
    let Some(root) = config.as_object_mut() else {
        return Ok(None);
    };

    let raw_profile = root.remove("launchSettingsProfile");
    let raw_file_path = root.remove("launchSettingsFilePath");

    let profile = match raw_profile {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err("`launchSettingsProfile` must be a non-empty string".to_string());
            }
            Some(trimmed.to_string())
        }
        Some(other) => {
            return Err(format!(
                "`launchSettingsProfile` must be a string, got {}",
                type_name(&other)
            ))
        }
    };

    let file_path = match raw_file_path {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err("`launchSettingsFilePath` must be a non-empty string".to_string());
            }
            Some(trimmed.to_string())
        }
        Some(other) => {
            return Err(format!(
                "`launchSettingsFilePath` must be a string, got {}",
                type_name(&other)
            ))
        }
    };

    match (file_path, profile) {
        (None, None) => Ok(None),
        (file_path, profile) => Ok(Some(LaunchSettingsSpec {
            file_path: file_path.unwrap_or_else(|| DEFAULT_LAUNCH_SETTINGS_PATH.to_string()),
            profile,
        })),
    }
}

/// Parses a `launchSettings.json` (JSONC) document and selects either the named
/// profile or the first profile with `"commandName": "Project"`.
pub fn parse_launch_settings_profile(
    content: &str,
    profile_name: Option<&str>,
    source_path: &str,
) -> Result<LaunchSettingsProfile, String> {
    let parsed = parse_jsonc(content)
        .map_err(|e| format!("failed to parse launchSettings file {source_path:?}: {e}"))?;

    let profiles = parsed
        .get("profiles")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!("launchSettings file {source_path:?} is missing a `profiles` object")
        })?;

    let mut available: Vec<&str> = profiles.keys().map(String::as_str).collect();
    available.sort();

    let (selected_name, selected_val) = if let Some(name) = profile_name {
        profiles
            .get(name)
            .map(|value| (name, value))
            .ok_or_else(|| {
                format!(
                    "launchSettings profile {name:?} was not found in {source_path:?}; \
                     available profiles: [{}]",
                    available.join(", ")
                )
            })?
    } else {
        profiles
            .iter()
            .find(|(_, value)| value.get("commandName").and_then(Value::as_str) == Some("Project"))
            .map(|(name, value)| (name.as_str(), value))
            .ok_or_else(|| {
                format!(
                    "no profile with `\"commandName\": \"Project\"` found in {source_path:?}; \
                     available profiles: [{}]",
                    available.join(", ")
                )
            })?
    };

    let profile_obj = selected_val.as_object().ok_or_else(|| {
        format!("launchSettings profile {selected_name:?} in {source_path:?} must be an object")
    })?;

    let mut environment_variables = Vec::new();
    match profile_obj.get("environmentVariables") {
        None | Some(Value::Null) => {}
        Some(Value::Object(map)) => {
            for (k, v) in map {
                match v {
                    Value::String(s) => environment_variables.push((k.clone(), s.clone())),
                    Value::Number(n) => environment_variables.push((k.clone(), n.to_string())),
                    Value::Bool(b) => environment_variables.push((k.clone(), b.to_string())),
                    Value::Null => {}
                    other => {
                        return Err(format!(
                            "`environmentVariables[\"{k}\"]` in launchSettings profile \
                             {selected_name:?} must be a string, got {}",
                            type_name(other)
                        ))
                    }
                }
            }
        }
        Some(other) => {
            return Err(format!(
                "`environmentVariables` in launchSettings profile {selected_name:?} must be an \
                 object, got {}",
                type_name(other)
            ))
        }
    }

    let application_url = match profile_obj.get("applicationUrl") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Some(other) => {
            return Err(format!(
                "`applicationUrl` in launchSettings profile {selected_name:?} must be a string, \
                 got {}",
                type_name(other)
            ))
        }
    };

    let command_line_args = match profile_obj.get("commandLineArgs") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let args = crate::locator::split_command_line(s);
            (!args.is_empty()).then_some(args)
        }
        Some(other) => {
            return Err(format!(
                "`commandLineArgs` in launchSettings profile {selected_name:?} must be a string, \
                 got {}",
                type_name(other)
            ))
        }
    };

    let working_directory = match profile_obj.get("workingDirectory") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Some(other) => {
            return Err(format!(
                "`workingDirectory` in launchSettings profile {selected_name:?} must be a \
                 string, got {}",
                type_name(other)
            ))
        }
    };

    Ok(LaunchSettingsProfile {
        environment_variables,
        application_url,
        command_line_args,
        working_directory,
    })
}

/// Merges `envFile` and `launchSettings.json` data into a launch configuration
/// with precedence: explicit `env`/`args`/`cwd` > `launchSettings` > `envFile`.
pub fn merge_launch_sources(
    config: &mut Value,
    env_file_vars: Option<&[(String, String)]>,
    launch_profile: Option<&LaunchSettingsProfile>,
) -> Result<(), String> {
    if env_file_vars.is_none() && launch_profile.is_none() {
        return Ok(());
    }

    let root = config
        .as_object_mut()
        .ok_or_else(|| "debug configuration must be a JSON object".to_string())?;

    let mut merged_env = Map::new();
    if let Some(vars) = env_file_vars {
        for (k, v) in vars {
            merged_env.insert(k.clone(), Value::String(v.clone()));
        }
    }

    if let Some(profile) = launch_profile {
        if let Some(url) = &profile.application_url {
            merged_env.insert("ASPNETCORE_URLS".to_string(), Value::String(url.clone()));
        }
        for (k, v) in &profile.environment_variables {
            merged_env.insert(k.clone(), Value::String(v.clone()));
        }
    }

    let explicit_env_was_object = match root.get("env") {
        None | Some(Value::Null) => false,
        Some(Value::Object(map)) => {
            for (k, v) in map {
                let s = v
                    .as_str()
                    .ok_or_else(|| format!("`env[\"{k}\"]` must be a string value"))?;
                merged_env.insert(k.clone(), Value::String(s.to_string()));
            }
            true
        }
        Some(_) => {
            return Err("`env` must be an object of string keys to string values".to_string())
        }
    };

    if !merged_env.is_empty() || explicit_env_was_object {
        root.insert("env".to_string(), Value::Object(merged_env));
    }

    if let Some(profile) = launch_profile {
        if matches!(root.get("args"), None | Some(Value::Null)) {
            if let Some(profile_args) = &profile.command_line_args {
                root.insert(
                    "args".to_string(),
                    Value::Array(profile_args.iter().cloned().map(Value::String).collect()),
                );
            }
        }

        let explicit_cwd_non_blank = root
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|s| !s.is_empty());
        if !explicit_cwd_non_blank
            && matches!(
                root.get("cwd"),
                None | Some(Value::Null) | Some(Value::String(_))
            )
        {
            if let Some(wd) = &profile.working_directory {
                root.insert("cwd".to_string(), Value::String(wd.clone()));
            }
        }
    }

    Ok(())
}

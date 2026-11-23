use crate::serde_json::Value;

use crate::config::jsonc::parse_jsonc;
/// Recursively substitutes VSCode variables (`${workspaceFolder}`, `${command:pickProcess}`)
/// with Zed equivalents.
pub fn substitute_macros(value: &mut Value, worktree_root: &str) {
    match value {
        Value::String(s) => {
            let mut text = s.replace("${workspaceFolder}", worktree_root);
            text = text.replace("${workspaceRoot}", worktree_root);
            if text == "${command:pickProcess}" {
                text = "${ZED_PICK_PID}".to_string();
            }
            *s = text;
        }
        Value::Array(arr) => {
            for item in arr {
                substitute_macros(item, worktree_root);
            }
        }
        Value::Object(map) => {
            for (_, v) in map {
                substitute_macros(v, worktree_root);
            }
        }
        _ => {}
    }
}

/// Extracts and translates a `coreclr` debug configuration from `.vscode/launch.json` content.
pub fn translate_launch_json(
    launch_json_content: &str,
    target_name: Option<&str>,
    worktree_root: &str,
) -> Result<Option<Value>, String> {
    let parsed = parse_jsonc(launch_json_content)
        .map_err(|e| format!("failed to parse launch.json: {e}"))?;

    let configurations = match parsed.get("configurations").and_then(Value::as_array) {
        Some(confs) => confs,
        None => return Ok(None),
    };

    let target_config = if let Some(name) = target_name {
        configurations.iter().find(|c| {
            c.get("name").and_then(Value::as_str) == Some(name)
                && c.get("type").and_then(Value::as_str) == Some("coreclr")
        })
    } else {
        configurations
            .iter()
            .find(|c| c.get("type").and_then(Value::as_str) == Some("coreclr"))
    };

    let Some(mut chosen) = target_config.cloned() else {
        return Ok(None);
    };

    substitute_macros(&mut chosen, worktree_root);
    Ok(Some(chosen))
}

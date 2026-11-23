use crate::serde_json::Value;

use crate::config::type_name;
/// Extracts and strips `envFile` from a debug configuration.
pub fn take_env_file(config: &mut Value) -> Result<Option<String>, String> {
    let Some(raw) = config
        .as_object_mut()
        .and_then(|object| object.remove("envFile"))
    else {
        return Ok(None);
    };
    match raw {
        Value::Null => Ok(None),
        Value::String(path) => {
            let trimmed = path.trim();
            if trimmed.is_empty() {
                Err("`envFile` must be a non-empty file path string".to_string())
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        other => Err(format!(
            "`envFile` must be a string file path (e.g. \".env\"), got {}",
            type_name(&other)
        )),
    }
}

/// Parses dotenv (`KEY=VALUE`) file contents into ordered `(key, value)` pairs.
///
/// Supports blank lines, `#` comments, optional `export ` prefixes, and
/// matching single or double quotes (preserving `#` and `=` inside quotes).
pub fn parse_env_file(content: &str) -> Result<Vec<(String, String)>, String> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut entries: Vec<(String, String)> = Vec::new();

    for (idx, raw_line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw_line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let after_export = if let Some(after) = trimmed.strip_prefix("export") {
            if after.starts_with(char::is_whitespace) {
                after.trim_start()
            } else {
                trimmed
            }
        } else {
            trimmed
        };

        let Some((raw_key, raw_val)) = after_export.split_once('=') else {
            return Err(format!(
                "invalid envFile syntax on line {line_no}: expected `KEY=VALUE`"
            ));
        };

        let key = raw_key.trim();
        if key.is_empty() || key.chars().any(char::is_whitespace) {
            return Err(format!(
                "invalid envFile variable name on line {line_no}: {key:?}"
            ));
        }

        let value = parse_env_value(raw_val, line_no)?;
        if let Some((_, existing)) = entries.iter_mut().find(|(k, _)| k == key) {
            *existing = value;
        } else {
            entries.push((key.to_string(), value));
        }
    }

    Ok(entries)
}

pub(super) fn parse_env_value(raw_val: &str, line_no: usize) -> Result<String, String> {
    let val = raw_val.trim_start();
    if let Some(rest) = val.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = rest.char_indices();
        let mut closed_at = None;
        while let Some((i, ch)) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some((_, 'n')) => out.push('\n'),
                    Some((_, 'r')) => out.push('\r'),
                    Some((_, 't')) => out.push('\t'),
                    Some((_, '"')) => out.push('"'),
                    Some((_, '\\')) => out.push('\\'),
                    Some((_, other)) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => {
                        return Err(format!(
                            "unterminated double quote in envFile on line {line_no}"
                        ))
                    }
                }
            } else if ch == '"' {
                closed_at = Some(i + ch.len_utf8());
                break;
            } else {
                out.push(ch);
            }
        }
        let Some(end) = closed_at else {
            return Err(format!(
                "unterminated double quote in envFile on line {line_no}"
            ));
        };
        let trailing = rest[end..].trim();
        if !trailing.is_empty() && !trailing.starts_with('#') {
            return Err(format!(
                "unexpected characters after closing quote in envFile on line {line_no}"
            ));
        }
        Ok(out)
    } else if let Some(rest) = val.strip_prefix('\'') {
        let Some((inner, trailing)) = rest.split_once('\'') else {
            return Err(format!(
                "unterminated single quote in envFile on line {line_no}"
            ));
        };
        let trailing = trailing.trim();
        if !trailing.is_empty() && !trailing.starts_with('#') {
            return Err(format!(
                "unexpected characters after closing quote in envFile on line {line_no}"
            ));
        }
        Ok(inner.to_string())
    } else {
        // Strip inline `# comment` only when `#` is preceded by whitespace.
        let mut end = val.len();
        let mut prev_ws = false;
        for (i, ch) in val.char_indices() {
            if ch == '#' && prev_ws {
                end = i;
                break;
            }
            prev_ws = ch.is_whitespace();
        }
        Ok(val[..end].trim_end().to_string())
    }
}

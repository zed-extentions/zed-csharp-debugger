/// Replaces Windows separators with `/` (both are accepted by `dotnet`,
/// netcoredbg and Windows path APIs).
pub fn normalize_separators(path: &str) -> String {
    path.replace('\\', "/")
}

/// Whether a path is absolute on either Windows (`C:\...`/`C:/...`, UNC) or
/// Unix (`/...`).
pub fn is_absolute_path(path: &str) -> bool {
    let path = normalize_separators(path);
    if path.starts_with('/') {
        return true;
    }
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Joins a base directory and a (possibly absolute) path, handling both
/// separator styles.
pub fn join_path(base: &str, path: &str) -> String {
    let path = normalize_separators(path.trim());
    if is_absolute_path(&path) {
        return path;
    }
    let path = path.strip_prefix("./").unwrap_or(&path);
    let base = normalize_separators(base.trim());
    let base = base.trim_end_matches('/');
    if base.is_empty() {
        return path.to_string();
    }
    if path.is_empty() || path == "." {
        return base.to_string();
    }
    format!("{base}/{path}")
}

/// The parent directory of a path with either separator style (`None` when
/// there is none, e.g. a bare file name or a root path).
pub fn parent_directory(path: &str) -> Option<String> {
    let normalized = normalize_separators(path.trim());
    let idx = normalized.rfind('/')?;
    if idx == 0 {
        return None;
    }
    Some(normalized[..idx].to_string())
}

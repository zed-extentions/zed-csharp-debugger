/// Normalizes a user-supplied file path (`envFile`, `launchSettingsFilePath`)
/// into a worktree-relative forward-slash path suitable for `Worktree::read_text_file`.
pub fn normalize_worktree_relative_path(
    raw: &str,
    worktree_root: &str,
    field_name: &str,
) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("`{field_name}` must not be empty"));
    }

    let mut path = trimmed.replace('\\', "/");
    for macro_prefix in [
        "${ZED_WORKTREE_ROOT}",
        "$ZED_WORKTREE_ROOT",
        "${workspaceFolder}",
        "${workspaceRoot}",
    ] {
        if path == macro_prefix {
            return Err(format!(
                "`{field_name}` ({raw:?}) must name a file inside the worktree"
            ));
        }
        if let Some(rest) = path.strip_prefix(&format!("{macro_prefix}/")) {
            path = rest.to_string();
            break;
        }
    }

    let root_norm = worktree_root.trim().replace('\\', "/");
    let root_trimmed = root_norm.trim_end_matches('/');
    if !root_trimmed.is_empty() {
        let is_windows_root = root_trimmed.len() >= 2 && root_trimmed.as_bytes()[1] == b':';
        let matches_exact = if is_windows_root {
            path.eq_ignore_ascii_case(root_trimmed)
        } else {
            path == root_trimmed
        };
        if matches_exact {
            return Err(format!(
                "`{field_name}` ({raw:?}) must name a file inside the worktree"
            ));
        }
        let prefix_len = root_trimmed.len();
        if path.len() > prefix_len && path.as_bytes()[prefix_len] == b'/' {
            let prefix_matches = if is_windows_root {
                path[..prefix_len].eq_ignore_ascii_case(root_trimmed)
            } else {
                &path[..prefix_len] == root_trimmed
            };
            if prefix_matches {
                path = path[prefix_len + 1..].to_string();
            }
        }
    }

    if crate::locator::is_absolute_path(&path) {
        return Err(format!(
            "`{field_name}` path {raw:?} must be inside the worktree ({worktree_root:?})"
        ));
    }

    let mut segments: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            if segments.pop().is_none() {
                return Err(format!(
                    "`{field_name}` path {raw:?} escapes the worktree root"
                ));
            }
        } else {
            segments.push(seg);
        }
    }

    if segments.is_empty() {
        return Err(format!("`{field_name}` must name a file path"));
    }

    Ok(segments.join("/"))
}

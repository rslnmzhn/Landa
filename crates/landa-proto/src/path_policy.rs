use std::path::{Component, Path, PathBuf};

use crate::error::ProtoError;

const MAX_SEGMENT_CHARS: usize = 120;
const MAX_RELATIVE_PATH_LEN: usize = 1024;

const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Validates and normalizes an untrusted relative path from a remote peer.
/// Rejects traversal (`..`), absolute paths, drive prefixes, NUL/control chars,
/// and Windows reserved device names.
pub fn validate_relative_path(raw: &str) -> Result<PathBuf, ProtoError> {
    if raw.is_empty() || raw.len() > MAX_RELATIVE_PATH_LEN {
        return Err(ProtoError::UnsafePath("empty or overly long path".into()));
    }
    if raw.chars().any(|c| c == '\0' || (c as u32) < 0x20) {
        return Err(ProtoError::UnsafePath("control characters in path".into()));
    }
    if raw.starts_with('/') || raw.starts_with('\\') {
        return Err(ProtoError::UnsafePath("absolute path not allowed".into()));
    }

    let normalized = raw.replace('\\', "/");
    let mut out = PathBuf::new();
    let mut count = 0usize;

    for segment in normalized.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(ProtoError::UnsafePath(format!(
                "invalid path segment: {segment:?}"
            )));
        }
        if segment.chars().count() > MAX_SEGMENT_CHARS {
            return Err(ProtoError::UnsafePath("path segment too long".into()));
        }
        if segment.contains(':')
            || segment.contains('<')
            || segment.contains('>')
            || segment.contains('"')
            || segment.contains('|')
            || segment.contains('?')
            || segment.contains('*')
        {
            return Err(ProtoError::UnsafePath(format!(
                "forbidden character in segment: {segment:?}"
            )));
        }
        if segment.ends_with(' ') || segment.ends_with('.') {
            return Err(ProtoError::UnsafePath(format!(
                "trailing dot or space in segment: {segment:?}"
            )));
        }
        let stem = segment.split('.').next().unwrap_or("").to_ascii_lowercase();
        if WINDOWS_RESERVED_NAMES.contains(&stem.as_str()) {
            return Err(ProtoError::UnsafePath(format!(
                "reserved device name in segment: {segment:?}"
            )));
        }
        out.push(segment);
        count += 1;
    }

    if count == 0 {
        return Err(ProtoError::UnsafePath("no path segments".into()));
    }

    // Double-check via std::path::Component
    for comp in out.components() {
        match comp {
            Component::Normal(_) => {}
            _ => return Err(ProtoError::UnsafePath("non-normal path component".into())),
        }
    }

    Ok(out)
}

/// Sanitizes an untrusted relative path into a safe relative PathBuf (for display/fallback).
/// Prefer `validate_relative_path` at protocol boundaries so malicious peers are rejected outright.
pub fn sanitize_relative_path(raw: &str) -> PathBuf {
    let normalized = raw.replace('\\', "/");
    let mut out = PathBuf::new();
    for part in normalized.split('/') {
        let cleaned = sanitize_segment(part.trim());
        if !cleaned.is_empty() && cleaned != "." && cleaned != ".." {
            out.push(cleaned);
        }
    }
    if out.as_os_str().is_empty() {
        out.push("file.bin");
    }
    out
}

fn sanitize_segment(input: &str) -> String {
    if input.is_empty() || input == "." || input == ".." {
        return String::new();
    }
    let mut value: String = input
        .chars()
        .filter(|&c| (c as u32) >= 0x20)
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            _ => c,
        })
        .collect();

    while value.ends_with(' ') || value.ends_with('.') {
        value.pop();
    }
    if value.is_empty() {
        return "_".to_string();
    }
    let stem = value.split('.').next().unwrap_or("").to_ascii_lowercase();
    if WINDOWS_RESERVED_NAMES.contains(&stem.as_str()) {
        value = format!("_{value}");
    }
    value.chars().take(MAX_SEGMENT_CHARS).collect()
}

/// Resolves `relative` inside `root` and verifies no symlink/canonical escape occurs.
pub fn resolve_within_root(root: &Path, relative: &Path) -> Result<PathBuf, ProtoError> {
    for comp in relative.components() {
        if !matches!(comp, Component::Normal(_)) {
            return Err(ProtoError::UnsafePath("non-normal component".into()));
        }
    }
    let canonical_root = root.canonicalize()?;
    let candidate = canonical_root.join(relative);
    if candidate.exists() {
        let canonical_candidate = candidate.canonicalize()?;
        if !canonical_candidate.starts_with(&canonical_root) {
            return Err(ProtoError::UnsafePath("symlink escape detected".into()));
        }
        return Ok(canonical_candidate);
    }
    // Check existing ancestor
    let mut ancestor = candidate.parent();
    while let Some(dir) = ancestor {
        if dir.exists() {
            let canonical_dir = dir.canonicalize()?;
            if !canonical_dir.starts_with(&canonical_root) {
                return Err(ProtoError::UnsafePath("ancestor escape detected".into()));
            }
            break;
        }
        ancestor = dir.parent();
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_reserved_names() {
        assert!(validate_relative_path("../secret.txt").is_err());
        assert!(validate_relative_path("a/../../etc/passwd").is_err());
        assert!(validate_relative_path("/etc/passwd").is_err());
        assert!(validate_relative_path("C:\\Windows\\win.ini").is_err());
        assert!(validate_relative_path("folder/CON.txt").is_err());
        assert!(validate_relative_path("folder/nul").is_err());
        assert!(validate_relative_path("bad\0name.txt").is_err());
        assert!(validate_relative_path("trailing.").is_err());
    }

    #[test]
    fn accepts_clean_nested_paths() {
        let p = validate_relative_path("photos/2026/vacation.jpg").unwrap();
        assert_eq!(p, PathBuf::from("photos").join("2026").join("vacation.jpg"));
    }
}

//! Pure helpers for local multi-git index-here: project slug normalize + file quality gates.
//! Shared by CLI and API; no I/O.

use sha2::{Digest, Sha256};
use std::path::{Component, Path};

/// Max file size for local index-here ingest (1 MiB).
pub const MAX_LOCAL_FILE_BYTES: u64 = 1_000_000;

/// Built-in allowlist (CLI/API default). Laptop can override via config TUI.
pub const DEFAULT_ALLOWED_EXTENSIONS: &[&str] = &[
    "md", "mdx", "astro", "ts", "tsx", "js", "jsx", "json", "yaml", "yml", "toml", "rs",
];

const DENIED_PATH_COMPONENTS: &[&str] = &[
    ".git",
    "node_modules",
    "dist",
    "build",
    "target",
    "coverage",
];

const LOCKFILE_NAMES: &[&str] = &[
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "cargo.lock",
];

/// Derive a stable project slug from an optional git remote URL/SCP origin and a directory basename.
///
/// Algorithm:
/// 1. From origin: path after host → last path segment → strip `.git`
/// 2. Else use `basename`
/// 3. Lowercase
/// 4. Replace non `[a-z0-9-]` with `-`
/// 5. Collapse `--`, trim `-`; empty → `"repo"`
#[must_use]
pub fn normalize_project_slug_from_origin(origin: Option<&str>, basename: &str) -> String {
    let raw = origin
        .and_then(extract_repo_name_from_origin)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| basename.to_owned());
    sanitize_slug(&raw)
}

/// Whether a repository-relative path + size should be considered for local index-here
/// using [`DEFAULT_ALLOWED_EXTENSIONS`].
#[must_use]
pub fn should_index_local_file(path: &str, size: u64) -> bool {
    should_index_local_file_with_extensions(path, size, DEFAULT_ALLOWED_EXTENSIONS)
}

/// Same as [`should_index_local_file`] with an explicit extension allowlist.
/// Extensions are compared case-insensitively without a leading dot.
/// Empty `allowed_extensions` falls back to the default list.
#[must_use]
pub fn should_index_local_file_with_extensions(
    path: &str,
    size: u64,
    allowed_extensions: &[&str],
) -> bool {
    if path.is_empty() || size > MAX_LOCAL_FILE_BYTES {
        return false;
    }
    let candidate = Path::new(path);
    if candidate.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return false;
    }
    if candidate.components().any(|component| {
        let Component::Normal(value) = component else {
            return false;
        };
        let value = value.to_string_lossy();
        DENIED_PATH_COMPONENTS
            .iter()
            .any(|denied| value.eq_ignore_ascii_case(denied))
    }) {
        return false;
    }

    let file_name = candidate
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if file_name == ".env"
        || file_name.starts_with(".env.")
        || LOCKFILE_NAMES
            .iter()
            .any(|lock| file_name.eq_ignore_ascii_case(lock))
    {
        return false;
    }

    let extension = candidate
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let allow = if allowed_extensions.is_empty() {
        DEFAULT_ALLOWED_EXTENSIONS
    } else {
        allowed_extensions
    };
    allow
        .iter()
        .any(|allowed| extension.eq_ignore_ascii_case(allowed.trim_start_matches('.')))
}

/// Drop empty / whitespace-only document bodies.
#[must_use]
pub fn content_is_indexable(content: &str) -> bool {
    !content.trim().is_empty()
}

/// SHA-256 hex of content UTF-8 bytes.
#[must_use]
pub fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn extract_repo_name_from_origin(origin: &str) -> Option<String> {
    let origin = origin.trim();
    if origin.is_empty() {
        return None;
    }

    // URL form: https://host/x/y/z.git or ssh://host/path/repo.git
    if let Ok(url) = url::Url::parse(origin) {
        let last = url
            .path_segments()
            .and_then(|mut segs| segs.next_back())
            .unwrap_or("")
            .trim_end_matches('/')
            .to_owned();
        let name = strip_git_suffix(&last);
        return if name.is_empty() { None } else { Some(name) };
    }

    // SCP form: git@host:path/to/repo.git
    if let Some((_, host_and_path)) = origin.split_once('@')
        && let Some((_, path)) = host_and_path.split_once(':')
    {
        let last = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(path)
            .trim_end_matches('/');
        let name = strip_git_suffix(last);
        return if name.is_empty() { None } else { Some(name) };
    }

    None
}

fn strip_git_suffix(name: &str) -> String {
    let name = name.trim();
    if name.len() > 4 && name.to_ascii_lowercase().ends_with(".git") {
        name[..name.len() - 4].to_owned()
    } else {
        name.to_owned()
    }
}

fn sanitize_slug(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut prev_dash = false;
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            if ch == '-' {
                if prev_dash {
                    continue;
                }
                prev_dash = true;
            } else {
                prev_dash = false;
            }
            out.push(ch);
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "repo".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_from_github_scp_origin() {
        assert_eq!(
            normalize_project_slug_from_origin(
                Some("git@github.com:nandocoeg2/fjulian.me.git"),
                "ignored"
            ),
            "fjulian-me"
        );
    }

    #[test]
    fn slug_from_selfhosted_scp_origin() {
        assert_eq!(
            normalize_project_slug_from_origin(Some("git@selfhosted:group/app.git"), "ignored"),
            "app"
        );
    }

    #[test]
    fn slug_from_https_origin() {
        assert_eq!(
            normalize_project_slug_from_origin(Some("https://gitlab.example/x/y/z.git"), "ignored"),
            "z"
        );
    }

    #[test]
    fn slug_from_basename_when_no_origin() {
        assert_eq!(normalize_project_slug_from_origin(None, "My App"), "my-app");
    }

    #[test]
    fn slug_empty_raw_becomes_repo() {
        assert_eq!(normalize_project_slug_from_origin(None, "---"), "repo");
        assert_eq!(normalize_project_slug_from_origin(None, ""), "repo");
    }

    #[test]
    fn slug_collapses_and_trims_dashes() {
        assert_eq!(
            normalize_project_slug_from_origin(None, "--Hello__World--"),
            "hello-world"
        );
    }

    #[test]
    fn gate_allows_supported_extensions() {
        for path in [
            "docs/runbook.md",
            "page.mdx",
            "src/page.astro",
            "src/a.ts",
            "src/a.tsx",
            "src/a.js",
            "src/a.jsx",
            "config.json",
            "config.yaml",
            "config.yml",
            "Cargo.toml",
            "src/lib.rs",
            "crates/foo/src/main.rs",
        ] {
            assert!(should_index_local_file(path, 100), "expected allow: {path}");
        }
        // target/ still denied even for .rs
        assert!(!should_index_local_file("target/debug/build.rs", 10));
    }

    #[test]
    fn gate_respects_custom_extension_allowlist() {
        assert!(!should_index_local_file_with_extensions(
            "src/lib.rs",
            100,
            &["md", "toml"]
        ));
        assert!(should_index_local_file_with_extensions(
            "src/lib.rs",
            100,
            &["rs"]
        ));
        assert!(should_index_local_file_with_extensions(
            "docs/a.md",
            100,
            &[".MD"]
        ));
        // empty → default list includes rs after product default
        assert!(should_index_local_file_with_extensions(
            "src/a.rs",
            100,
            &[]
        ));
    }

    #[test]
    fn gate_denies_path_components() {
        for path in [
            ".git/config",
            "node_modules/pkg/readme.md",
            "dist/generated.json",
            "build/out.ts",
            "target/debug/build.rs",
            "coverage/lcov.info.md",
        ] {
            assert!(
                !should_index_local_file(path, 10),
                "expected deny component: {path}"
            );
        }
    }

    #[test]
    fn gate_denies_env_and_lockfiles() {
        assert!(!should_index_local_file(".env", 10));
        assert!(!should_index_local_file(".env.local", 10));
        assert!(!should_index_local_file("package-lock.json", 10));
        assert!(!should_index_local_file("pnpm-lock.yaml", 10));
        assert!(!should_index_local_file("yarn.lock", 10));
        assert!(!should_index_local_file("Cargo.lock", 10));
    }

    #[test]
    fn gate_denies_oversized_and_bad_paths() {
        assert!(!should_index_local_file(
            "docs/runbook.md",
            MAX_LOCAL_FILE_BYTES + 1
        ));
        assert!(should_index_local_file(
            "docs/runbook.md",
            MAX_LOCAL_FILE_BYTES
        ));
        assert!(!should_index_local_file("../escape.md", 10));
        assert!(!should_index_local_file("/abs.md", 10));
        assert!(!should_index_local_file("", 10));
        assert!(!should_index_local_file("assets/logo.png", 10));
    }

    #[test]
    fn content_indexable_drops_whitespace_only() {
        assert!(!content_is_indexable(""));
        assert!(!content_is_indexable("   \n\t  "));
        assert!(content_is_indexable("hello"));
    }

    #[test]
    fn content_hash_is_sha256_hex_of_utf8_bytes() {
        let hash = content_hash("hello");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        // echo -n hello | shasum -a 256
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_ne!(content_hash("hello"), content_hash("hello "));
    }
}

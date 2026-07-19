//! CLI `index-here`: discover local git roots, gate tracked files, upload to edge.
//!
//! POST `{edge}/api/v1/agent/index-local` with Bearer agent token.

use anyhow::{bail, Context, Result};
use queria_ingestion::local_index_gates::{
    content_hash, content_is_indexable, should_index_local_file, MAX_LOCAL_FILE_BYTES,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Default nested-git scan depth.
pub const DEFAULT_DEPTH: u32 = 4;

/// Soft payload limits matching server (`MAX_ROOTS=20`, `MAX_FILES_PER_REQUEST=500`).
const MAX_ROOTS_PER_REQUEST: usize = 20;
const MAX_FILES_PER_REQUEST: usize = 500;
/// Stay under ~4 MiB JSON body before request overhead.
const MAX_BATCH_BYTES: usize = 4 * 1024 * 1024;

const DEFAULT_EDGE_URL: &str = "http://127.0.0.1:17674";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredRoot {
    pub path: PathBuf,
    pub origin_url: Option<String>,
    pub commit_sha: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexableFile {
    pub path: String,
    pub content: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootFilePlan {
    pub root: DiscoveredRoot,
    pub accepted: Vec<IndexableFile>,
    pub skipped: u32,
}

#[derive(Debug, Serialize)]
struct IndexLocalRequest {
    roots: Vec<IndexLocalRootPayload>,
}

#[derive(Debug, Serialize)]
struct IndexLocalRootPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    origin_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_path_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit_sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    files: Vec<IndexLocalFilePayload>,
}

#[derive(Debug, Serialize)]
struct IndexLocalFilePayload {
    path: String,
    content: String,
    content_hash: String,
}

#[derive(Debug, Deserialize)]
struct IndexLocalResponse {
    job_ids: Vec<String>,
    roots: Vec<RootStatsResponse>,
}

#[derive(Debug, Deserialize)]
struct RootStatsResponse {
    project_slug: String,
    #[allow(dead_code)]
    project_id: String,
    files_accepted: u32,
    files_skipped: u32,
}

/// Entry point for `queria-cli index-here`.
pub async fn run(
    token_env: &str,
    edge_url_env: &str,
    depth: u32,
    yes: bool,
    dry_run: bool,
) -> Result<()> {
    let cwd = std::env::current_dir().context("resolve current working directory")?;
    let roots = discover_git_roots(&cwd, depth)?;
    if roots.is_empty() {
        bail!("no git roots found under {} (depth={depth})", cwd.display());
    }

    let all_paths: Vec<PathBuf> = roots.iter().map(|r| r.path.clone()).collect();
    let plans: Vec<RootFilePlan> = roots
        .into_iter()
        .map(|root| plan_root_files(root, &all_paths))
        .collect::<Result<Vec<_>>>()?;

    print_discovery_summary(&plans);

    if dry_run {
        println!("dry-run: no upload");
        return Ok(());
    }

    if plans.len() > 1 && !yes {
        bail!(
            "pass --yes to index {} roots (or --dry-run to list only)",
            plans.len()
        );
    }

    let token = read_token(token_env)?;
    let edge_base = edge_base_url(edge_url_env);
    let endpoint = format!(
        "{}/api/v1/agent/index-local",
        edge_base.trim_end_matches('/')
    );

    upload_plans(&endpoint, &token, &plans).await
}

fn read_token(token_env: &str) -> Result<String> {
    let token = std::env::var(token_env).with_context(|| {
        format!("missing agent token: set env var {token_env} (use --token-env to change name)")
    })?;
    let token = token.trim().to_owned();
    if token.is_empty() {
        bail!("env var {token_env} is empty");
    }
    Ok(token)
}

fn edge_base_url(edge_url_env: &str) -> String {
    std::env::var(edge_url_env)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_EDGE_URL.to_owned())
}

fn print_discovery_summary(plans: &[RootFilePlan]) {
    println!("discovered {} git root(s):", plans.len());
    for plan in plans {
        let origin = plan
            .root
            .origin_url
            .as_deref()
            .unwrap_or("(no origin)");
        let branch = plan.root.branch.as_deref().unwrap_or("?");
        let sha = plan
            .root
            .commit_sha
            .as_deref()
            .map(|s| {
                if s.len() > 12 {
                    &s[..12]
                } else {
                    s
                }
            })
            .unwrap_or("?");
        println!(
            "  {}  origin={}  branch={}  HEAD={}  accept={} skip={}",
            plan.root.path.display(),
            origin,
            branch,
            sha,
            plan.accepted.len(),
            plan.skipped
        );
    }
}

/// Discover git work tree roots starting at `start` (usually cwd).
///
/// 1. If `start` is inside a work tree → add that toplevel
/// 2. Walk descendants depth ≤ `depth`; on `.git` dir/file → root; do not walk inside nested root
/// 3. Dedupe canonical paths
pub fn discover_git_roots(start: &Path, depth: u32) -> Result<Vec<DiscoveredRoot>> {
    let mut ordered: Vec<PathBuf> = Vec::new();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();

    if let Some(toplevel) = git_toplevel(start) {
        push_unique_root(&mut ordered, &mut seen, &toplevel);
    }

    walk_for_nested_roots(start, depth, 0, &mut ordered, &mut seen)?;

    let mut roots = Vec::with_capacity(ordered.len());
    for path in ordered {
        roots.push(inspect_root(path)?);
    }
    Ok(roots)
}

fn push_unique_root(ordered: &mut Vec<PathBuf>, seen: &mut BTreeSet<PathBuf>, path: &Path) {
    let canonical = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf());
    if seen.insert(canonical.clone()) {
        ordered.push(canonical);
    }
}

fn walk_for_nested_roots(
    dir: &Path,
    max_depth: u32,
    current_depth: u32,
    ordered: &mut Vec<PathBuf>,
    seen: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    if current_depth > max_depth {
        return Ok(());
    }

    // At depth 0 we may already be inside a root; still scan children up to max_depth.
    // When we find a nested root at or below max_depth, record it and do not descend into it
    // for the parent's file listing (each root has its own ls-files later).
    if current_depth > 0 {
        if let Some(root) = git_root_if_present(dir) {
            push_unique_root(ordered, seen, &root);
            return Ok(());
        }
    }

    if current_depth == max_depth {
        // Still detect a git root at exactly max depth, but do not walk deeper.
        if let Some(root) = git_root_if_present(dir) {
            push_unique_root(ordered, seen, &root);
        }
        return Ok(());
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Skip VCS internals if present under a non-root dir.
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n == ".git")
        {
            continue;
        }
        walk_for_nested_roots(&path, max_depth, current_depth + 1, ordered, seen)?;
    }
    Ok(())
}

/// If `dir` itself is a git work tree root (contains `.git`), return its path.
fn git_root_if_present(dir: &Path) -> Option<PathBuf> {
    let git = dir.join(".git");
    if git.is_dir() || git.is_file() {
        return Some(dir.to_path_buf());
    }
    None
}

fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if text.is_empty() {
        return None;
    }
    Some(PathBuf::from(text))
}

fn inspect_root(path: PathBuf) -> Result<DiscoveredRoot> {
    let origin_url = git_stdout(&path, &["remote", "get-url", "origin"]).ok();
    let commit_sha = git_stdout(&path, &["rev-parse", "HEAD"]).ok();
    let branch = match git_stdout(&path, &["symbolic-ref", "--short", "HEAD"]) {
        Ok(b) => Some(b),
        Err(_) => Some("detached".to_owned()),
    };
    Ok(DiscoveredRoot {
        path,
        origin_url,
        commit_sha,
        branch,
    })
}

fn git_stdout(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("git {} in {}", args.join(" "), cwd.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git {} failed in {}: {}",
            args.join(" "),
            cwd.display(),
            stderr.trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Relative path prefixes of other discovered roots that are strict children of `root`.
/// Used to drop parent `ls-files` entries that belong to a nested git root.
pub fn nested_path_prefixes(root: &Path, all_roots: &[PathBuf]) -> Vec<String> {
    let root_c = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut prefixes = Vec::new();
    for other in all_roots {
        let other_c = other.canonicalize().unwrap_or_else(|_| other.to_path_buf());
        if other_c == root_c {
            continue;
        }
        if let Ok(rel) = other_c.strip_prefix(&root_c) {
            let s = rel.to_string_lossy().replace('\\', "/");
            if !s.is_empty() {
                prefixes.push(s);
            }
        }
    }
    prefixes.sort();
    prefixes.dedup();
    prefixes
}

/// True if `rel` is the nested root path itself or any file under it.
pub fn path_under_nested_prefixes(rel: &str, prefixes: &[String]) -> bool {
    let rel = rel.replace('\\', "/");
    prefixes.iter().any(|p| rel == *p || rel.starts_with(&format!("{p}/")))
}

/// List tracked files under root and apply client quality gates.
///
/// `all_roots` is the full discover set for this run: paths under nested git
/// roots are skipped for the parent so they are only planned for the nested root.
pub fn plan_root_files(root: DiscoveredRoot, all_roots: &[PathBuf]) -> Result<RootFilePlan> {
    let nested = nested_path_prefixes(&root.path, all_roots);
    let tracked = git_ls_files(&root.path)?;
    let mut accepted = Vec::new();
    let mut skipped = 0_u32;

    for rel in tracked {
        if path_under_nested_prefixes(&rel, &nested) {
            skipped += 1;
            continue;
        }
        match process_tracked_file(&root.path, &rel) {
            Some(file) => accepted.push(file),
            None => skipped += 1,
        }
    }

    Ok(RootFilePlan {
        root,
        accepted,
        skipped,
    })
}

fn git_ls_files(root: &Path) -> Result<Vec<String>> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .with_context(|| format!("git ls-files in {}", root.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git ls-files failed in {}: {}", root.display(), stderr.trim());
    }
    let paths = output
        .stdout
        .split(|b| *b == 0)
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect();
    Ok(paths)
}

/// Process one repo-relative tracked path: size gate, read utf-8, content gate, hash.
/// Returns None if skipped (count as skip).
pub fn process_tracked_file(root: &Path, rel_path: &str) -> Option<IndexableFile> {
    if rel_path.is_empty() {
        return None;
    }
    let abs = root.join(rel_path);
    let meta = fs::metadata(&abs).ok()?;
    if !meta.is_file() {
        return None;
    }
    let size = meta.len();
    if size > MAX_LOCAL_FILE_BYTES {
        return None;
    }
    if !should_index_local_file(rel_path, size) {
        return None;
    }
    let content = fs::read_to_string(&abs).ok()?;
    if !content_is_indexable(&content) {
        return None;
    }
    // Re-check size after reading (UTF-8 body length for consistency with API).
    if (content.len() as u64) > MAX_LOCAL_FILE_BYTES {
        return None;
    }
    if !should_index_local_file(rel_path, content.len() as u64) {
        return None;
    }
    let hash = content_hash(&content);
    Some(IndexableFile {
        path: rel_path.to_owned(),
        content,
        content_hash: hash,
    })
}

/// Pure gate counting for tests without filesystem: given (path, size, content) outcomes.
pub fn count_gate_outcomes(files: &[(&str, u64, &str)]) -> (u32, u32) {
    let mut accept = 0_u32;
    let mut skip = 0_u32;
    for (path, size, content) in files {
        if should_index_local_file(path, *size)
            && content_is_indexable(content)
            && (content.len() as u64) <= MAX_LOCAL_FILE_BYTES
        {
            accept += 1;
        } else {
            skip += 1;
        }
    }
    (accept, skip)
}

async fn upload_plans(endpoint: &str, token: &str, plans: &[RootFilePlan]) -> Result<()> {
    let client = reqwest::Client::new();
    let batches = build_batches(plans);
    if batches.is_empty() {
        eprintln!("no accepted files to upload");
        return Ok(());
    }

    let total_batches = batches.len();
    for (i, batch) in batches.into_iter().enumerate() {
        let n_files: usize = batch.iter().map(|r| r.files.len()).sum();
        let n_roots = batch.len();
        let body = IndexLocalRequest { roots: batch };
        let response = client
            .post(endpoint)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {endpoint}"))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .unwrap_or_else(|_| String::from("<unreadable body>"));

        if !status.is_success() {
            let snippet: String = text.chars().take(500).collect();
            eprintln!("index-local failed: HTTP {status}");
            eprintln!("{snippet}");
            bail!("index-local HTTP {status}");
        }

        let parsed: IndexLocalResponse = serde_json::from_str(&text)
            .with_context(|| format!("parse index-local response: {}", truncate(&text, 200)))?;

        eprint_progress(i + 1, total_batches, n_roots, n_files, &parsed);
    }

    Ok(())
}

fn eprint_progress(
    batch_no: usize,
    total_batches: usize,
    n_roots: usize,
    n_files: usize,
    resp: &IndexLocalResponse,
) {
    let mut out = io::stderr();
    let _ = writeln!(
        out,
        "batch {batch_no}/{total_batches}: sent {n_roots} root(s), {n_files} file(s); job_ids={:?}",
        resp.job_ids
    );
    for r in &resp.roots {
        let _ = writeln!(
            out,
            "  project={} accepted={} skipped={}",
            r.project_slug, r.files_accepted, r.files_skipped
        );
    }
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Split plans into request batches under root/file/byte limits.
fn build_batches(plans: &[RootFilePlan]) -> Vec<Vec<IndexLocalRootPayload>> {
    let mut batches: Vec<Vec<IndexLocalRootPayload>> = Vec::new();
    let mut current: Vec<IndexLocalRootPayload> = Vec::new();
    let mut files_in_batch = 0_usize;
    let mut bytes_in_batch = 0_usize;

    for plan in plans {
        // Roots with zero accepted files still inform discover dry-run; skip empty upload.
        if plan.accepted.is_empty() {
            continue;
        }

        // Chunk this root's files if a single root exceeds limits.
        let mut file_chunks: Vec<Vec<&IndexableFile>> = Vec::new();
        let mut chunk: Vec<&IndexableFile> = Vec::new();
        let mut chunk_bytes = 0_usize;
        for file in &plan.accepted {
            let file_bytes = estimate_file_bytes(file);
            if !chunk.is_empty()
                && (chunk.len() >= MAX_FILES_PER_REQUEST
                    || chunk_bytes + file_bytes > MAX_BATCH_BYTES)
            {
                file_chunks.push(std::mem::take(&mut chunk));
                chunk_bytes = 0;
            }
            chunk.push(file);
            chunk_bytes += file_bytes;
        }
        if !chunk.is_empty() {
            file_chunks.push(chunk);
        }

        for files in file_chunks {
            let payload_files: Vec<IndexLocalFilePayload> = files
                .iter()
                .map(|f| IndexLocalFilePayload {
                    path: f.path.clone(),
                    content: f.content.clone(),
                    content_hash: f.content_hash.clone(),
                })
                .collect();
            let chunk_file_count = payload_files.len();
            let chunk_bytes: usize = files.iter().map(|f| estimate_file_bytes(f)).sum();

            let needs_new = !current.is_empty()
                && (current.len() >= MAX_ROOTS_PER_REQUEST
                    || files_in_batch + chunk_file_count > MAX_FILES_PER_REQUEST
                    || bytes_in_batch + chunk_bytes > MAX_BATCH_BYTES);

            if needs_new {
                batches.push(std::mem::take(&mut current));
                files_in_batch = 0;
                bytes_in_batch = 0;
            }

            current.push(IndexLocalRootPayload {
                origin_url: plan.root.origin_url.clone(),
                local_path_hint: Some(plan.root.path.display().to_string()),
                commit_sha: plan.root.commit_sha.clone(),
                branch: plan.root.branch.clone(),
                files: payload_files,
            });
            files_in_batch += chunk_file_count;
            bytes_in_batch += chunk_bytes;
        }
    }

    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

fn estimate_file_bytes(file: &IndexableFile) -> usize {
    // Rough JSON overhead: path + hash + content + keys.
    file.path.len() + file.content.len() + file.content_hash.len() + 64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command as StdCommand;

    fn git_available() -> bool {
        StdCommand::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn init_repo(dir: &Path) {
        fs::create_dir_all(dir).expect("mkdir");
        let status = StdCommand::new("git")
            .args(["init", "-q"])
            .current_dir(dir)
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");
        // Identity for commit-less tree; ls-files works without commits.
        let _ = StdCommand::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(dir)
            .status();
        let _ = StdCommand::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .status();
    }

    fn write_and_add(repo: &Path, rel: &str, body: &str) {
        let path = repo.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir parent");
        }
        fs::write(&path, body).expect("write");
        let status = StdCommand::new("git")
            .args(["add", rel])
            .current_dir(repo)
            .status()
            .expect("git add");
        assert!(status.success(), "git add {rel}");
    }

    #[test]
    fn discover_includes_cwd_toplevel_and_nested() {
        if !git_available() {
            eprintln!("skip: git not available");
            return;
        }
        let tmp = tempfile_dir("discover-nested");
        let parent = tmp.join("workspace");
        let nested = parent.join("services").join("api");
        init_repo(&parent);
        init_repo(&nested);
        write_and_add(&parent, "README.md", "# parent");
        write_and_add(&nested, "src/main.ts", "export {}");

        let roots = discover_git_roots(&parent, 4).expect("discover");
        let paths: Vec<String> = roots
            .iter()
            .map(|r| r.path.canonicalize().unwrap_or_else(|_| r.path.clone()))
            .map(|p| p.display().to_string())
            .collect();

        let parent_c = parent.canonicalize().unwrap();
        let nested_c = nested.canonicalize().unwrap();
        assert!(
            paths.iter().any(|p| p == &parent_c.display().to_string()),
            "expected parent root in {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p == &nested_c.display().to_string()),
            "expected nested root in {paths:?}"
        );
        // Deduped
        assert_eq!(paths.len(), paths.iter().collect::<BTreeSet<_>>().len());
    }

    #[test]
    fn discover_dedupes_same_toplevel() {
        if !git_available() {
            eprintln!("skip: git not available");
            return;
        }
        let tmp = tempfile_dir("discover-dedupe");
        let repo = tmp.join("one");
        init_repo(&repo);
        write_and_add(&repo, "a.md", "x");

        let roots = discover_git_roots(&repo, 2).expect("discover");
        assert_eq!(roots.len(), 1);
        assert_eq!(
            roots[0].path.canonicalize().unwrap(),
            repo.canonicalize().unwrap()
        );
    }

    #[test]
    fn plan_root_accepts_md_skips_denied() {
        if !git_available() {
            eprintln!("skip: git not available");
            return;
        }
        let tmp = tempfile_dir("plan-gates");
        let repo = tmp.join("r");
        init_repo(&repo);
        write_and_add(&repo, "docs/ok.md", "hello");
        write_and_add(&repo, "node_modules/pkg/readme.md", "nope");
        write_and_add(&repo, "logo.png", "not-text-but-binaryish");

        let root_path = repo.canonicalize().unwrap();
        let root = inspect_root(root_path.clone()).expect("inspect");
        let plan = plan_root_files(root, &[root_path]).expect("plan");
        assert_eq!(plan.accepted.len(), 1);
        assert_eq!(plan.accepted[0].path, "docs/ok.md");
        assert!(plan.skipped >= 2);
        assert_eq!(
            plan.accepted[0].content_hash,
            content_hash("hello")
        );
    }

    #[test]
    fn parent_plan_skips_paths_under_nested_git_root() {
        if !git_available() {
            eprintln!("skip: git not available");
            return;
        }
        let tmp = tempfile_dir("nested-filter");
        let parent = tmp.join("workspace");
        let nested = parent.join("services").join("api");
        init_repo(&parent);
        init_repo(&nested);
        write_and_add(&parent, "docs/parent.md", "# parent only");
        // Parent monorepo wrongly tracks nested paths (submodule-like).
        write_and_add(&parent, "services/api/src/main.ts", "export const bad = 1;");
        write_and_add(&nested, "src/main.ts", "export const good = 1;");
        write_and_add(&nested, "README.md", "nested");

        let parent_c = parent.canonicalize().unwrap();
        let nested_c = nested.canonicalize().unwrap();
        let all = vec![parent_c.clone(), nested_c.clone()];

        let prefixes = nested_path_prefixes(&parent_c, &all);
        assert!(
            prefixes.iter().any(|p| p == "services/api" || p.ends_with("services/api")),
            "expected nested prefix, got {prefixes:?}"
        );
        assert!(path_under_nested_prefixes("services/api/src/main.ts", &prefixes));
        assert!(!path_under_nested_prefixes("docs/parent.md", &prefixes));

        let parent_root = inspect_root(parent_c.clone()).expect("inspect parent");
        let parent_plan = plan_root_files(parent_root, &all).expect("plan parent");
        assert!(
            parent_plan
                .accepted
                .iter()
                .all(|f| !f.path.starts_with("services/api")),
            "parent must not accept nested paths: {:?}",
            parent_plan.accepted.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
        assert!(
            parent_plan.accepted.iter().any(|f| f.path == "docs/parent.md"),
            "parent should still accept own files"
        );

        let nested_root = inspect_root(nested_c.clone()).expect("inspect nested");
        let nested_plan = plan_root_files(nested_root, &all).expect("plan nested");
        assert!(
            nested_plan.accepted.iter().any(|f| f.path == "src/main.ts" || f.path == "README.md"),
            "nested root should accept own tracked files: {:?}",
            nested_plan.accepted.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn sibling_roots_do_not_filter_each_other() {
        if !git_available() {
            eprintln!("skip: git not available");
            return;
        }
        let tmp = tempfile_dir("sibling-filter");
        let a = tmp.join("a");
        let b = tmp.join("b");
        init_repo(&a);
        init_repo(&b);
        write_and_add(&a, "a.md", "aaa");
        write_and_add(&b, "b.md", "bbb");
        let ac = a.canonicalize().unwrap();
        let bc = b.canonicalize().unwrap();
        let all = vec![ac.clone(), bc.clone()];
        assert!(nested_path_prefixes(&ac, &all).is_empty());
        let plan_a = plan_root_files(inspect_root(ac).unwrap(), &all).unwrap();
        assert_eq!(plan_a.accepted.len(), 1);
        assert_eq!(plan_a.accepted[0].path, "a.md");
    }

    #[test]
    fn count_gate_outcomes_pure() {
        let (a, s) = count_gate_outcomes(&[
            ("docs/a.md", 5, "hello"),
            ("node_modules/x.md", 5, "hello"),
            ("docs/b.md", 5, "   "),
            ("docs/c.ts", 2, "x"),
        ]);
        assert_eq!(a, 2);
        assert_eq!(s, 2);
    }

    #[test]
    fn build_batches_respects_file_limit() {
        let root = DiscoveredRoot {
            path: PathBuf::from("/tmp/repo"),
            origin_url: Some("git@h:a.git".to_owned()),
            commit_sha: Some("abc".to_owned()),
            branch: Some("main".to_owned()),
        };
        let mut accepted = Vec::new();
        for i in 0..501 {
            accepted.push(IndexableFile {
                path: format!("f{i}.md"),
                content: "x".to_owned(),
                content_hash: content_hash("x"),
            });
        }
        let plan = RootFilePlan {
            root,
            accepted,
            skipped: 0,
        };
        let batches = build_batches(&[plan]);
        // Single root chunked into multiple request roots / batches under 500 files.
        let max_files = batches
            .iter()
            .map(|b| b.iter().map(|r| r.files.len()).sum::<usize>())
            .max()
            .unwrap_or(0);
        assert!(max_files <= MAX_FILES_PER_REQUEST);
        let total: usize = batches.iter().map(|b| b.iter().map(|r| r.files.len()).sum::<usize>()).sum();
        assert_eq!(total, 501);
    }

    #[test]
    fn multi_root_without_yes_message_format() {
        // Document exit contract used by run(): multi root needs --yes.
        let msg = format!(
            "pass --yes to index {} roots (or --dry-run to list only)",
            3
        );
        assert!(msg.contains("--yes"));
        assert!(msg.contains("3"));
    }

    fn tempfile_dir(label: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "queria-index-here-{}-{}-{}",
            label,
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).expect("temp base");
        base
    }
}

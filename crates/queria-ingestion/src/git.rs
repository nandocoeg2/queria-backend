use async_trait::async_trait;
use mockall::automock;
use queria_core::{QueriaError, QueriaResult};
use std::path::{Component, Path, PathBuf};
use tokio::process::Command;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitFile {
    pub path: String,
    pub content: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitSnapshot {
    pub commit_sha: String,
    pub branch: String,
    pub files: Vec<GitFile>,
}

#[automock]
#[async_trait]
pub trait GitRepositoryGateway: Send + Sync {
    async fn snapshot(&self, repository_path: &Path) -> QueriaResult<GitSnapshot>;
}

#[derive(Clone, Debug)]
pub struct GitSecurityPolicy {
    allowed_roots: Vec<PathBuf>,
    allowed_ssh_hosts: Vec<String>,
    allowed_ssh_repositories: Vec<String>,
    excluded_directories: Vec<String>,
    max_file_bytes: u64,
}

impl GitSecurityPolicy {
    pub fn new(
        allowed_roots: Vec<PathBuf>,
        allowed_ssh_hosts: Vec<String>,
        allowed_ssh_repositories: Vec<String>,
        excluded_directories: Vec<String>,
        max_file_bytes: u64,
    ) -> QueriaResult<Self> {
        if allowed_roots.is_empty()
            || allowed_ssh_hosts.is_empty()
            || allowed_ssh_repositories.is_empty()
            || max_file_bytes == 0
        {
            return Err(QueriaError::Config(
                "Git ingestion allowlists and file limit must not be empty".to_owned(),
            ));
        }
        let allowed_roots = allowed_roots
            .into_iter()
            .map(|path| {
                path.canonicalize().map_err(|error| {
                    QueriaError::Config(format!("Git allowed root is unavailable: {error}"))
                })
            })
            .collect::<QueriaResult<Vec<_>>>()?;

        Ok(Self {
            allowed_roots,
            allowed_ssh_hosts,
            allowed_ssh_repositories,
            excluded_directories,
            max_file_bytes,
        })
    }

    pub fn validate_repository(&self, path: &Path, uri: &str) -> QueriaResult<PathBuf> {
        let canonical = path.canonicalize().map_err(|_| {
            QueriaError::Validation("Git repository path does not exist".to_owned())
        })?;
        if !self
            .allowed_roots
            .iter()
            .any(|allowed| canonical == *allowed || canonical.starts_with(allowed))
        {
            return Err(QueriaError::PermissionDenied);
        }

        self.validate_uri(&canonical, uri)?;
        Ok(canonical)
    }

    pub fn should_index_file(&self, path: &str, size: u64) -> bool {
        let candidate = Path::new(path);
        if path.is_empty()
            || size > self.max_file_bytes
            || candidate.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return false;
        }
        if candidate.components().any(|component| {
            let Component::Normal(value) = component else {
                return false;
            };
            let value = value.to_string_lossy();
            self.excluded_directories
                .iter()
                .any(|excluded| value.eq_ignore_ascii_case(excluded))
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
            || matches!(
                file_name.as_str(),
                "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock" | "cargo.lock"
            )
        {
            return false;
        }

        matches!(
            candidate
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str(),
            "md" | "mdx" | "astro" | "ts" | "tsx" | "js" | "jsx" | "json" | "yaml" | "yml" | "toml"
        )
    }

    fn validate_uri(&self, canonical_path: &Path, uri: &str) -> QueriaResult<()> {
        if let Ok(url) = url::Url::parse(uri) {
            if url.scheme() == "file" {
                let uri_path = url.to_file_path().map_err(|()| {
                    QueriaError::Validation("invalid file Git source URI".to_owned())
                })?;
                let uri_path = uri_path.canonicalize().map_err(|_| {
                    QueriaError::Validation("file Git source URI does not exist".to_owned())
                })?;
                return if uri_path == canonical_path {
                    Ok(())
                } else {
                    Err(QueriaError::PermissionDenied)
                };
            }
            if url.scheme() == "ssh" {
                let host = url
                    .host_str()
                    .ok_or_else(|| QueriaError::Validation("SSH URI has no host".to_owned()))?;
                return self.validate_ssh_parts(host, url.path().trim_start_matches('/'));
            }
        }

        let (_, host_and_path) = uri.split_once('@').ok_or_else(|| {
            QueriaError::Validation("Git source URI must use file or SSH".to_owned())
        })?;
        let (host, repository) = host_and_path
            .split_once(':')
            .ok_or_else(|| QueriaError::Validation("invalid SSH Git URI".to_owned()))?;
        self.validate_ssh_parts(host, repository)
    }

    fn validate_ssh_parts(&self, host: &str, repository: &str) -> QueriaResult<()> {
        let allowed_host = self
            .allowed_ssh_hosts
            .iter()
            .any(|allowed| host.eq_ignore_ascii_case(allowed));
        let allowed_repository = self
            .allowed_ssh_repositories
            .iter()
            .any(|allowed| repository == allowed);
        if allowed_host && allowed_repository {
            Ok(())
        } else {
            Err(QueriaError::PermissionDenied)
        }
    }
}

#[derive(Clone, Debug)]
pub struct GitCliGateway {
    policy: GitSecurityPolicy,
}

impl GitCliGateway {
    #[must_use]
    pub fn new(policy: GitSecurityPolicy) -> Self {
        Self { policy }
    }

    async fn git_output(repository_path: &Path, args: &[&str]) -> QueriaResult<Vec<u8>> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository_path)
            .args(args)
            .output()
            .await
            .map_err(|error| {
                QueriaError::Infrastructure(format!("failed to start Git: {error}"))
            })?;
        if !output.status.success() {
            return Err(QueriaError::Infrastructure(format!(
                "Git command failed with status {}",
                output.status
            )));
        }
        Ok(output.stdout)
    }
}

#[async_trait]
impl GitRepositoryGateway for GitCliGateway {
    async fn snapshot(&self, repository_path: &Path) -> QueriaResult<GitSnapshot> {
        let commit_sha =
            String::from_utf8(Self::git_output(repository_path, &["rev-parse", "HEAD"]).await?)
                .map_err(|_| {
                    QueriaError::Infrastructure("Git returned non-UTF8 commit SHA".to_owned())
                })?
                .trim()
                .to_owned();
        if commit_sha.len() != 40 || !commit_sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(QueriaError::Infrastructure(
                "Git returned an invalid commit SHA".to_owned(),
            ));
        }
        let branch =
            match Self::git_output(repository_path, &["symbolic-ref", "--short", "HEAD"]).await {
                Ok(output) => String::from_utf8(output)
                    .map_err(|_| QueriaError::Infrastructure("Git branch is not UTF-8".to_owned()))?
                    .trim()
                    .to_owned(),
                Err(_) => "detached".to_owned(),
            };
        let tracked = Self::git_output(repository_path, &["ls-files", "-z"]).await?;
        let mut files = Vec::new();
        for raw_path in tracked
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            let path = std::str::from_utf8(raw_path)
                .map_err(|_| QueriaError::Validation("tracked Git path is not UTF-8".to_owned()))?;
            let full_path = repository_path.join(path);
            let metadata = tokio::fs::symlink_metadata(&full_path)
                .await
                .map_err(|error| {
                    QueriaError::Infrastructure(format!("failed to inspect tracked file: {error}"))
                })?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || !self.policy.should_index_file(path, metadata.len())
            {
                continue;
            }
            let resolved = full_path.canonicalize().map_err(|error| {
                QueriaError::Infrastructure(format!("failed to resolve tracked file: {error}"))
            })?;
            if !resolved.starts_with(repository_path) {
                return Err(QueriaError::PermissionDenied);
            }
            let content = tokio::fs::read_to_string(&resolved)
                .await
                .map_err(|error| {
                    QueriaError::Validation(format!("tracked source file is not UTF-8: {error}"))
                })?;
            files.push(GitFile {
                path: path.to_owned(),
                content,
                size_bytes: metadata.len(),
            });
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(GitSnapshot {
            commit_sha,
            branch,
            files,
        })
    }
}

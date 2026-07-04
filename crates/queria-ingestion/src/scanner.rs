use async_trait::async_trait;
use mockall::automock;
use queria_core::{QueriaError, QueriaResult};
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

#[automock]
#[async_trait]
pub trait SecretScanner: Send + Sync {
    async fn scan(&self, repository_path: &Path) -> QueriaResult<()>;
}

#[derive(Clone, Debug)]
pub struct TruffleHogScanner {
    executable: String,
    include_paths_file: PathBuf,
    exclude_paths_file: PathBuf,
    timeout: Duration,
}

impl TruffleHogScanner {
    #[must_use]
    pub fn new(
        executable: String,
        include_paths_file: PathBuf,
        exclude_paths_file: PathBuf,
        timeout: Duration,
    ) -> Self {
        Self {
            executable,
            include_paths_file,
            exclude_paths_file,
            timeout,
        }
    }
}

#[async_trait]
impl SecretScanner for TruffleHogScanner {
    async fn scan(&self, repository_path: &Path) -> QueriaResult<()> {
        let command = Command::new(&self.executable)
            .arg("filesystem")
            .arg("--json")
            .arg("--no-update")
            .arg("--fail")
            .arg("--fail-on-scan-errors")
            .arg("--force-skip-binaries")
            .arg("--force-skip-archives")
            .arg("--include-paths")
            .arg(&self.include_paths_file)
            .arg("--exclude-paths")
            .arg(&self.exclude_paths_file)
            .arg(repository_path)
            .output();
        let output = tokio::time::timeout(self.timeout, command)
            .await
            .map_err(|_| QueriaError::Infrastructure("TruffleHog scan timed out".to_owned()))?
            .map_err(|error| {
                QueriaError::Infrastructure(format!("failed to start TruffleHog: {error}"))
            })?;
        if !output.status.success() {
            return Err(QueriaError::Validation(format!(
                "TruffleHog rejected the source with status {}",
                output.status
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanner_requires_bounded_scope_files() {
        let scanner = TruffleHogScanner::new(
            "trufflehog".to_owned(),
            PathBuf::from("include.txt"),
            PathBuf::from("exclude.txt"),
            Duration::from_secs(300),
        );

        assert_eq!(scanner.include_paths_file, PathBuf::from("include.txt"));
        assert_eq!(scanner.exclude_paths_file, PathBuf::from("exclude.txt"));
        assert_eq!(scanner.timeout, Duration::from_secs(300));
    }
}

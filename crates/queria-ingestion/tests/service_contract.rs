use async_trait::async_trait;
use queria_core::{QueriaError, QueriaResult};
use queria_ingestion::git::{GitFile, GitRepositoryGateway, GitSecurityPolicy, GitSnapshot};
use queria_ingestion::scanner::SecretScanner;
use queria_ingestion::service::{GitIngestionService, GitIngestionSource};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct FailScanner;
#[async_trait]
impl SecretScanner for FailScanner {
    async fn scan(&self, _repository_path: &Path) -> QueriaResult<()> {
        Err(QueriaError::Validation("secret found".to_owned()))
    }
}

struct OkScanner;
#[async_trait]
impl SecretScanner for OkScanner {
    async fn scan(&self, _repository_path: &Path) -> QueriaResult<()> {
        Ok(())
    }
}

struct TrackingGit {
    called: Arc<AtomicBool>,
}
#[async_trait]
impl GitRepositoryGateway for TrackingGit {
    async fn snapshot(&self, _repository_path: &Path) -> QueriaResult<GitSnapshot> {
        self.called.store(true, Ordering::SeqCst);
        panic!("git should not be called after secret scan failure");
    }
}

struct FixtureGit {
    calls: Arc<AtomicUsize>,
}
#[async_trait]
impl GitRepositoryGateway for FixtureGit {
    async fn snapshot(&self, _repository_path: &Path) -> QueriaResult<GitSnapshot> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(GitSnapshot {
            commit_sha: "a".repeat(40),
            branch: "main".to_owned(),
            files: vec![
                GitFile {
                    path: "src/page.astro".to_owned(),
                    content: "<h1>Page</h1>\n<p>Body</p>\n".to_owned(),
                    size_bytes: 31,
                },
                GitFile {
                    path: "README.md".to_owned(),
                    content: "# Project\nDocs\n".to_owned(),
                    size_bytes: 15,
                },
            ],
        })
    }
}

#[tokio::test]
async fn failed_secret_scan_stops_before_git_read() {
    let fixture = fixture_path("scan-failure");
    let policy = policy(&fixture);
    let git_called = Arc::new(AtomicBool::new(false));
    let git = TrackingGit {
        called: Arc::clone(&git_called),
    };
    let service = GitIngestionService::new(git, FailScanner, policy, 3, 1);

    let result = service.prepare(source(&fixture)).await;

    assert!(result.is_err());
    assert!(!git_called.load(Ordering::SeqCst));
    fs::remove_dir_all(fixture).expect("fixture should be removable");
}

#[tokio::test]
async fn verified_snapshot_becomes_sorted_deterministic_manifest() {
    let fixture = fixture_path("manifest");
    let policy = policy(&fixture);
    let git_calls = Arc::new(AtomicUsize::new(0));
    let git = FixtureGit {
        calls: Arc::clone(&git_calls),
    };
    let service = GitIngestionService::new(git, OkScanner, policy, 3, 1);

    let first = service
        .prepare(source(&fixture))
        .await
        .expect("manifest should prepare");

    assert_eq!(first.commit_sha, "a".repeat(40));
    assert_eq!(first.files[0].path, "README.md");
    assert_eq!(first.files[1].path, "src/page.astro");
    assert!(first.trusted_auto_approve);
    assert!(!first.content_hash.is_empty());
    assert_eq!(
        first.files[0].knowledge[0].chunks[0].citation_path,
        "README.md"
    );
    assert_eq!(git_calls.load(Ordering::SeqCst), 1);
    fs::remove_dir_all(fixture).expect("fixture should be removable");
}

fn fixture_path(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("queria-service-{name}-{}", std::process::id()));
    fs::create_dir_all(&path).expect("fixture should exist");
    path
}

fn policy(path: &std::path::Path) -> GitSecurityPolicy {
    GitSecurityPolicy::new(
        vec![path.to_path_buf()],
        vec!["github.com".to_owned()],
        vec!["nandocoeg2/fjulian.me.git".to_owned()],
        vec!["node_modules".to_owned()],
        1_000_000,
    )
    .expect("policy should build")
}

fn source(path: &std::path::Path) -> GitIngestionSource {
    GitIngestionSource {
        path: path.to_path_buf(),
        uri: "git@github.com:nandocoeg2/fjulian.me.git".to_owned(),
        trusted_auto_approve: true,
    }
}

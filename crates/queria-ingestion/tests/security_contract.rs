use queria_ingestion::git::GitSecurityPolicy;
use std::fs;

#[test]
fn repository_path_and_ssh_uri_must_be_allowlisted() {
    let temp = std::env::temp_dir().join(format!("queria-policy-{}", std::process::id()));
    let allowed = temp.join("allowed");
    let denied = temp.join("denied");
    fs::create_dir_all(&allowed).expect("allowed fixture should exist");
    fs::create_dir_all(&denied).expect("denied fixture should exist");
    let policy = GitSecurityPolicy::new(
        vec![allowed.clone()],
        vec!["github.com".to_owned()],
        vec!["nandocoeg2/fjulian.me.git".to_owned()],
        vec!["node_modules".to_owned(), "dist".to_owned()],
        1024,
    )
    .expect("policy should be valid");

    assert!(
        policy
            .validate_repository(&allowed, "git@github.com:nandocoeg2/fjulian.me.git")
            .is_ok()
    );
    assert!(
        policy
            .validate_repository(&denied, "git@github.com:nandocoeg2/fjulian.me.git")
            .is_err()
    );
    assert!(
        policy
            .validate_repository(&allowed, "git@gitlab.com:nandocoeg2/fjulian.me.git")
            .is_err()
    );
    assert!(
        policy
            .validate_repository(&allowed, "git@github.com:other/repo.git")
            .is_err()
    );

    fs::remove_dir_all(temp).expect("fixture should be removable");
}

#[test]
fn file_policy_excludes_generated_sensitive_and_large_files() {
    let policy = GitSecurityPolicy::new(
        vec![std::env::temp_dir()],
        vec!["github.com".to_owned()],
        vec!["nandocoeg2/fjulian.me.git".to_owned()],
        vec![
            "node_modules".to_owned(),
            "dist".to_owned(),
            "target".to_owned(),
        ],
        100,
    )
    .expect("policy should be valid");

    assert!(policy.should_index_file("docs/runbook.md", 100));
    assert!(policy.should_index_file("src/page.astro", 50));
    assert!(!policy.should_index_file("node_modules/pkg/readme.md", 10));
    assert!(!policy.should_index_file("dist/generated.json", 10));
    assert!(!policy.should_index_file(".env", 10));
    assert!(!policy.should_index_file("keys/deploy.pem", 10));
    assert!(!policy.should_index_file("docs/runbook.md", 101));
    assert!(!policy.should_index_file("assets/logo.png", 10));
}

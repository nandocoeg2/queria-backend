use crate::git::{GitRepositoryGateway, GitSecurityPolicy};
use crate::model::{PreparedFile, PreparedGitManifest, PreparedKnowledge};
use crate::parser::{chunk_sections, parse_document};
use crate::scanner::SecretScanner;
use queria_core::QueriaResult;
use sha2::{Digest, Sha256};

pub struct GitIngestionService<G, S> {
    git: G,
    scanner: S,
    policy: GitSecurityPolicy,
    max_chunk_lines: usize,
    overlap_lines: usize,
}

impl<G, S> GitIngestionService<G, S>
where
    G: GitRepositoryGateway,
    S: SecretScanner,
{
    pub fn new(
        git: G,
        scanner: S,
        policy: GitSecurityPolicy,
        max_chunk_lines: usize,
        overlap_lines: usize,
    ) -> Self {
        Self {
            git,
            scanner,
            policy,
            max_chunk_lines,
            overlap_lines,
        }
    }

    pub async fn prepare(&self, source: GitIngestionSource) -> QueriaResult<PreparedGitManifest> {
        let repository_path = self.policy.validate_repository(&source.path, &source.uri)?;
        self.scanner.scan(&repository_path).await?;
        let mut snapshot = self.git.snapshot(&repository_path).await?;
        snapshot
            .files
            .sort_by(|left, right| left.path.cmp(&right.path));

        let mut files = Vec::with_capacity(snapshot.files.len());
        for file in snapshot.files {
            let parsed =
                parse_document(&file.path, &file.content).map_err(|error| match error {
                    queria_core::QueriaError::Validation(message) => {
                        queria_core::QueriaError::Validation(format!("{}: {message}", file.path))
                    }
                    other => other,
                })?;
            let file_hash = hash_parts(&[file.content.as_bytes()]);
            let knowledge = parsed
                .sections
                .iter()
                .map(|section| {
                    let chunks = chunk_sections(
                        &file.path,
                        std::slice::from_ref(section),
                        self.max_chunk_lines,
                        self.overlap_lines,
                    )?;
                    Ok(PreparedKnowledge {
                        stable_key: hash_parts(&[file.path.as_bytes(), section.key.as_bytes()]),
                        title: section.title.clone(),
                        body: section.body.clone(),
                        category: parsed.parser.clone(),
                        line_start: section.line_start,
                        line_end: section.line_end,
                        chunks,
                    })
                })
                .collect::<QueriaResult<Vec<_>>>()?;
            files.push(PreparedFile {
                path: file.path,
                parser: parsed.parser,
                content_hash: file_hash,
                size_bytes: file.size_bytes,
                knowledge,
            });
        }

        let mut manifest_hasher = Sha256::new();
        for file in &files {
            manifest_hasher.update(file.path.as_bytes());
            manifest_hasher.update([0]);
            manifest_hasher.update(file.content_hash.as_bytes());
            manifest_hasher.update([0]);
        }

        Ok(PreparedGitManifest {
            commit_sha: snapshot.commit_sha,
            branch: snapshot.branch,
            content_hash: hex_digest(manifest_hasher.finalize().as_slice()),
            trusted_auto_approve: source.trusted_auto_approve,
            files,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitIngestionSource {
    pub path: std::path::PathBuf,
    pub uri: String,
    pub trusted_auto_approve: bool,
}

fn hash_parts(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
        hasher.update([0]);
    }
    hex_digest(hasher.finalize().as_slice())
}

fn hex_digest(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

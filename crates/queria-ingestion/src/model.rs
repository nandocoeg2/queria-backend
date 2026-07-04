#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedDocument {
    pub parser: String,
    pub sections: Vec<ParsedSection>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedSection {
    pub key: String,
    pub title: String,
    pub body: String,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedChunk {
    pub stable_key: String,
    pub title: String,
    pub body: String,
    pub chunk_index: usize,
    pub line_start: usize,
    pub line_end: usize,
    pub citation_path: String,
    pub content_hash: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PreparedGitManifest {
    pub commit_sha: String,
    pub branch: String,
    pub content_hash: String,
    pub trusted_auto_approve: bool,
    pub files: Vec<PreparedFile>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedFile {
    pub path: String,
    pub parser: String,
    pub content_hash: String,
    pub size_bytes: u64,
    pub knowledge: Vec<PreparedKnowledge>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedKnowledge {
    pub stable_key: String,
    pub title: String,
    pub body: String,
    pub category: String,
    pub line_start: usize,
    pub line_end: usize,
    pub chunks: Vec<ParsedChunk>,
}

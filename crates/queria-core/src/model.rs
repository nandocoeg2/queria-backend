use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeScope {
    Global,
    Project,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeStatus {
    Draft,
    Proposed,
    Approved,
    Rejected,
    Deprecated,
    Superseded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    GitRepo,
    MarkdownDocs,
    ManualNote,
    IncidentReport,
    Sop,
    Config,
}

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
    /// Project-scoped agent memory lane (dual-lane Slice A). Not trusted/global.
    Scratch,
    /// Local index-here / hybrid multi-git items pending human review. Not trusted.
    NeedsReview,
}

impl KnowledgeStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Proposed => "proposed",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Deprecated => "deprecated",
            Self::Superseded => "superseded",
            Self::Scratch => "scratch",
            Self::NeedsReview => "needs_review",
        }
    }

    /// User-facing label (Admin/UI). Wire/DB form remains [`Self::as_str`].
    pub const fn display_label(self) -> &'static str {
        match self {
            Self::Draft => "Draft",
            Self::Proposed => "Proposed",
            Self::Approved => "Approved",
            Self::Rejected => "Rejected",
            Self::Deprecated => "Deprecated",
            Self::Superseded => "Superseded",
            Self::Scratch => "Scratch",
            Self::NeedsReview => "Needs review",
        }
    }

    /// Lane for agent retrieve is derived from status (no separate trust_lane column).
    pub const fn is_scratch_lane(self) -> bool {
        matches!(self, Self::Scratch)
    }

    pub const fn is_trusted_lane(self) -> bool {
        matches!(self, Self::Approved)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_status_includes_scratch_variant() {
        assert_eq!(KnowledgeStatus::Scratch.as_str(), "scratch");
        assert!(KnowledgeStatus::Scratch.is_scratch_lane());
        assert!(!KnowledgeStatus::Scratch.is_trusted_lane());
        assert!(KnowledgeStatus::Approved.is_trusted_lane());
        assert!(!KnowledgeStatus::Approved.is_scratch_lane());
        assert!(!KnowledgeStatus::Proposed.is_scratch_lane());
        assert!(!KnowledgeStatus::Draft.is_scratch_lane());
    }

    #[test]
    fn knowledge_status_includes_needs_review_variant() {
        assert_eq!(KnowledgeStatus::NeedsReview.as_str(), "needs_review");
        assert_eq!(KnowledgeStatus::NeedsReview.display_label(), "Needs review");
        assert!(!KnowledgeStatus::NeedsReview.is_scratch_lane());
        assert!(!KnowledgeStatus::NeedsReview.is_trusted_lane());
    }

    #[test]
    fn knowledge_status_scratch_serializes_snake_case() {
        let json = serde_json::to_string(&KnowledgeStatus::Scratch).expect("serialize");
        assert_eq!(json, "\"scratch\"");
        let parsed: KnowledgeStatus = serde_json::from_str("\"scratch\"").expect("deserialize");
        assert_eq!(parsed, KnowledgeStatus::Scratch);
    }

    #[test]
    fn knowledge_status_needs_review_serializes_snake_case() {
        let json = serde_json::to_string(&KnowledgeStatus::NeedsReview).expect("serialize");
        assert_eq!(json, "\"needs_review\"");
        let parsed: KnowledgeStatus =
            serde_json::from_str("\"needs_review\"").expect("deserialize");
        assert_eq!(parsed, KnowledgeStatus::NeedsReview);
    }

    #[test]
    fn knowledge_status_preserves_existing_variants() {
        for (status, expected) in [
            (KnowledgeStatus::Draft, "draft"),
            (KnowledgeStatus::Proposed, "proposed"),
            (KnowledgeStatus::Approved, "approved"),
            (KnowledgeStatus::Rejected, "rejected"),
            (KnowledgeStatus::Deprecated, "deprecated"),
            (KnowledgeStatus::Superseded, "superseded"),
            (KnowledgeStatus::Scratch, "scratch"),
            (KnowledgeStatus::NeedsReview, "needs_review"),
        ] {
            assert_eq!(status.as_str(), expected);
            let json = serde_json::to_string(&status).expect("serialize");
            assert_eq!(json, format!("\"{expected}\""));
        }
    }
}

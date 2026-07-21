use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolPermission {
    RetrieveContext,
    SearchKnowledge,
    ProposeMemory,
    /// Direct write to project-scoped scratch lane (not in default_agent_tools).
    IndexMemory,
    /// Local multi-git index-here upload (not in default_agent_tools).
    IndexLocal,
    /// Privileged needs_review list/promote/reject (not in default_agent_tools).
    ManageNeedsReview,
    ListProjects,
    GetSource,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentTokenPermissions {
    pub allow_global_knowledge: bool,
    pub project_slugs: Vec<String>,
    pub tools: Vec<AgentToolPermission>,
}

impl AgentTokenPermissions {
    #[must_use]
    pub fn can_call(&self, tool: &AgentToolPermission) -> bool {
        self.tools.contains(tool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_memory_serializes_as_snake_case() {
        let json = serde_json::to_string(&AgentToolPermission::IndexMemory)
            .expect("serialize IndexMemory");
        assert_eq!(json, "\"index_memory\"");
        let back: AgentToolPermission =
            serde_json::from_str("\"index_memory\"").expect("deserialize index_memory");
        assert_eq!(back, AgentToolPermission::IndexMemory);
    }

    #[test]
    fn can_call_index_memory_only_when_granted() {
        let without = AgentTokenPermissions {
            allow_global_knowledge: false,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: vec![
                AgentToolPermission::RetrieveContext,
                AgentToolPermission::ProposeMemory,
            ],
        };
        assert!(!without.can_call(&AgentToolPermission::IndexMemory));
        assert!(without.can_call(&AgentToolPermission::ProposeMemory));
        assert!(without.can_call(&AgentToolPermission::RetrieveContext));

        let with = AgentTokenPermissions {
            allow_global_knowledge: false,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: vec![
                AgentToolPermission::RetrieveContext,
                AgentToolPermission::ProposeMemory,
                AgentToolPermission::IndexMemory,
            ],
        };
        assert!(with.can_call(&AgentToolPermission::IndexMemory));
        assert!(with.can_call(&AgentToolPermission::ProposeMemory));
    }

    #[test]
    fn index_memory_coexists_with_propose_memory() {
        let tools = [
            AgentToolPermission::RetrieveContext,
            AgentToolPermission::ProposeMemory,
            AgentToolPermission::IndexMemory,
        ];
        assert!(tools.contains(&AgentToolPermission::IndexMemory));
        assert!(tools.contains(&AgentToolPermission::ProposeMemory));
        assert_ne!(
            AgentToolPermission::IndexMemory,
            AgentToolPermission::ProposeMemory
        );
    }

    #[test]
    fn index_local_serializes_as_snake_case() {
        let json =
            serde_json::to_string(&AgentToolPermission::IndexLocal).expect("serialize IndexLocal");
        assert_eq!(json, "\"index_local\"");
        let back: AgentToolPermission =
            serde_json::from_str("\"index_local\"").expect("deserialize index_local");
        assert_eq!(back, AgentToolPermission::IndexLocal);
    }

    #[test]
    fn can_call_index_local_only_when_granted() {
        let without = AgentTokenPermissions {
            allow_global_knowledge: false,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: vec![
                AgentToolPermission::RetrieveContext,
                AgentToolPermission::IndexMemory,
            ],
        };
        assert!(!without.can_call(&AgentToolPermission::IndexLocal));

        let with = AgentTokenPermissions {
            allow_global_knowledge: false,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: vec![
                AgentToolPermission::RetrieveContext,
                AgentToolPermission::IndexLocal,
            ],
        };
        assert!(with.can_call(&AgentToolPermission::IndexLocal));
        assert!(!with.can_call(&AgentToolPermission::IndexMemory));
    }

    #[test]
    fn index_local_not_same_as_index_memory() {
        assert_ne!(
            AgentToolPermission::IndexLocal,
            AgentToolPermission::IndexMemory
        );
    }

    #[test]
    fn manage_needs_review_serializes_as_snake_case() {
        let json = serde_json::to_string(&AgentToolPermission::ManageNeedsReview)
            .expect("serialize ManageNeedsReview");
        assert_eq!(json, "\"manage_needs_review\"");
        let back: AgentToolPermission = serde_json::from_str("\"manage_needs_review\"")
            .expect("deserialize manage_needs_review");
        assert_eq!(back, AgentToolPermission::ManageNeedsReview);
    }

    #[test]
    fn can_call_manage_needs_review_only_when_granted() {
        let without = AgentTokenPermissions {
            allow_global_knowledge: false,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: vec![
                AgentToolPermission::RetrieveContext,
                AgentToolPermission::IndexLocal,
            ],
        };
        assert!(!without.can_call(&AgentToolPermission::ManageNeedsReview));

        let with = AgentTokenPermissions {
            allow_global_knowledge: false,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: vec![
                AgentToolPermission::RetrieveContext,
                AgentToolPermission::ManageNeedsReview,
            ],
        };
        assert!(with.can_call(&AgentToolPermission::ManageNeedsReview));
    }
}

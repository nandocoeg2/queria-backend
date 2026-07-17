use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolPermission {
    RetrieveContext,
    SearchKnowledge,
    ProposeMemory,
    /// Direct write to project-scoped scratch lane (not in default_agent_tools).
    IndexMemory,
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
}

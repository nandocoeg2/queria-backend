use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolPermission {
    RetrieveContext,
    SearchKnowledge,
    ProposeMemory,
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

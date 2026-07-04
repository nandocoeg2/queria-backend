use queria_auth::permissions::{AgentTokenPermissions, AgentToolPermission};
use serde_json::{Value, json};

pub fn tool_definitions(permissions: &AgentTokenPermissions) -> Vec<Value> {
    tool_specs()
        .into_iter()
        .filter(|(permission, _)| permissions.can_call(permission))
        .map(|(_, definition)| definition)
        .collect()
}

pub fn permission_for_tool(name: &str) -> Option<AgentToolPermission> {
    match name {
        "retrieve_context" => Some(AgentToolPermission::RetrieveContext),
        "search_knowledge" => Some(AgentToolPermission::SearchKnowledge),
        "propose_memory" => Some(AgentToolPermission::ProposeMemory),
        "list_projects" => Some(AgentToolPermission::ListProjects),
        "get_source" => Some(AgentToolPermission::GetSource),
        _ => None,
    }
}

fn tool_specs() -> Vec<(AgentToolPermission, Value)> {
    vec![
        (
            AgentToolPermission::RetrieveContext,
            json!({
                "name": "retrieve_context",
                "title": "Retrieve Context",
                "description": "Retrieve approved project and optional global Queria knowledge with citations.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project_id": { "type": "string", "description": "Queria project UUID." },
                        "query": { "type": "string", "description": "Question or task context to retrieve for." },
                        "include_global": { "type": "boolean", "description": "Include global knowledge when the token allows it." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 20 }
                    },
                    "required": ["project_id", "query"],
                    "additionalProperties": false
                }
            }),
        ),
        (
            AgentToolPermission::SearchKnowledge,
            json!({
                "name": "search_knowledge",
                "title": "Search Knowledge",
                "description": "Search approved Queria knowledge for a project and return matching chunks.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project_id": { "type": "string" },
                        "query": { "type": "string" },
                        "include_global": { "type": "boolean" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 20 }
                    },
                    "required": ["project_id", "query"],
                    "additionalProperties": false
                }
            }),
        ),
        (
            AgentToolPermission::ProposeMemory,
            json!({
                "name": "propose_memory",
                "title": "Propose Memory",
                "description": "Propose a new project knowledge item for human approval.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project_slug": { "type": "string" },
                        "title": { "type": "string" },
                        "body": { "type": "string" },
                        "category": { "type": "string" },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    },
                    "required": ["project_slug", "title", "body", "category"],
                    "additionalProperties": false
                }
            }),
        ),
        (
            AgentToolPermission::ListProjects,
            json!({
                "name": "list_projects",
                "title": "List Projects",
                "description": "List projects accessible to the current Queria agent token.",
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": false
                }
            }),
        ),
        (
            AgentToolPermission::GetSource,
            json!({
                "name": "get_source",
                "title": "Get Source",
                "description": "Get a source document registry entry accessible to the current token.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "source_document_id": { "type": "string" }
                    },
                    "required": ["source_document_id"],
                    "additionalProperties": false
                }
            }),
        ),
    ]
}

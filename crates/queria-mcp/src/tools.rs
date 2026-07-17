use queria_core::auth::permissions::{AgentTokenPermissions, AgentToolPermission};
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
        "index_memory" => Some(AgentToolPermission::IndexMemory),
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
            AgentToolPermission::IndexMemory,
            json!({
                "name": "index_memory",
                "title": "Index Memory",
                "description": "Index project-scoped scratch memory for immediate dual-lane retrieve. Does not write trusted or global knowledge.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project_id": { "type": "string", "description": "Queria project UUID." },
                        "project_slug": { "type": "string", "description": "Queria project slug (alternative to project_id)." },
                        "body": { "type": "string", "description": "Scratch memory body text (required)." },
                        "title": { "type": "string", "description": "Optional short title." },
                        "category": { "type": "string", "description": "Optional category label." },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional freeform tags."
                        }
                    },
                    "required": ["body"],
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

#[cfg(test)]
mod tests {
    use super::*;
    use queria_core::auth::agent_token::default_agent_tools;

    fn tool_names(permissions: &AgentTokenPermissions) -> Vec<String> {
        tool_definitions(permissions)
            .into_iter()
            .map(|definition| definition["name"].as_str().expect("tool name").to_owned())
            .collect()
    }

    #[test]
    fn retrieve_context_is_available_only_when_token_allows_it() {
        let permissions = AgentTokenPermissions {
            allow_global_knowledge: true,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: vec![AgentToolPermission::RetrieveContext],
        };

        let tools = tool_definitions(&permissions);

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "retrieve_context");
        assert_eq!(
            permission_for_tool("retrieve_context"),
            Some(AgentToolPermission::RetrieveContext)
        );
    }

    #[test]
    fn retrieve_context_is_hidden_when_token_lacks_permission() {
        let permissions = AgentTokenPermissions {
            allow_global_knowledge: true,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: vec![AgentToolPermission::ListProjects],
        };

        let tools = tool_definitions(&permissions);

        assert!(
            tools
                .iter()
                .all(|definition| definition["name"] != "retrieve_context")
        );
    }

    /// VAL-DL-001 / VAL-CROSS-005: tools/list hides index_memory without IndexMemory.
    #[test]
    fn index_memory_is_hidden_without_index_memory_permission() {
        let permissions = AgentTokenPermissions {
            allow_global_knowledge: true,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: default_agent_tools(),
        };

        let names = tool_names(&permissions);
        assert!(
            !names.iter().any(|name| name == "index_memory"),
            "legacy propose-only tools must not list index_memory: {names:?}"
        );
        assert!(names.iter().any(|name| name == "propose_memory"));
        assert!(names.iter().any(|name| name == "retrieve_context"));
    }

    /// VAL-DL-002 / VAL-DL-045: tools/list shows index_memory with schema when granted.
    #[test]
    fn index_memory_is_listed_with_schema_when_granted() {
        let permissions = AgentTokenPermissions {
            allow_global_knowledge: false,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: vec![
                AgentToolPermission::RetrieveContext,
                AgentToolPermission::IndexMemory,
            ],
        };

        let tools = tool_definitions(&permissions);
        let index = tools
            .iter()
            .find(|definition| definition["name"] == "index_memory")
            .expect("index_memory must appear when IndexMemory is granted");

        let description = index["description"].as_str().unwrap_or_default();
        assert!(!description.is_empty());
        assert!(index["inputSchema"].is_object());
        assert!(
            index["inputSchema"]["properties"]["body"].is_object(),
            "body is required in product contract"
        );
        let props = &index["inputSchema"]["properties"];
        assert!(
            props.get("project_id").is_some() || props.get("project_slug").is_some(),
            "project selector required in schema"
        );
        assert_eq!(
            permission_for_tool("index_memory"),
            Some(AgentToolPermission::IndexMemory)
        );
    }

    /// VAL-DL-005 / VAL-DL-006: legacy retrieve + propose remain without IndexMemory.
    #[test]
    fn legacy_token_keeps_retrieve_and_propose_without_index_memory() {
        let permissions = AgentTokenPermissions {
            allow_global_knowledge: true,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: default_agent_tools(),
        };
        let names = tool_names(&permissions);
        assert!(names.iter().any(|name| name == "retrieve_context"));
        assert!(names.iter().any(|name| name == "propose_memory"));
        assert!(names.iter().any(|name| name == "search_knowledge"));
        assert!(!names.iter().any(|name| name == "index_memory"));
    }

    /// VAL-DL-053: IndexMemory is additive with ProposeMemory.
    #[test]
    fn index_memory_coexists_with_propose_memory_on_tools_list() {
        let permissions = AgentTokenPermissions {
            allow_global_knowledge: false,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: vec![
                AgentToolPermission::RetrieveContext,
                AgentToolPermission::ProposeMemory,
                AgentToolPermission::IndexMemory,
            ],
        };
        let names = tool_names(&permissions);
        assert!(names.iter().any(|name| name == "index_memory"));
        assert!(names.iter().any(|name| name == "propose_memory"));
        assert!(names.iter().any(|name| name == "retrieve_context"));
    }

    /// VAL-DL-004: unknown tool name has no permission mapping.
    #[test]
    fn unknown_tool_has_no_permission() {
        assert_eq!(permission_for_tool("no_such_tool"), None);
        assert_eq!(permission_for_tool("promote_memory"), None);
    }

    #[test]
    fn index_memory_only_token_does_not_list_propose() {
        let permissions = AgentTokenPermissions {
            allow_global_knowledge: false,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: vec![AgentToolPermission::IndexMemory],
        };
        let names = tool_names(&permissions);
        assert_eq!(names, vec!["index_memory".to_owned()]);
    }
}

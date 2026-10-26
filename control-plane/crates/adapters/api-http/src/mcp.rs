//! MCP server (Streamable HTTP): expose Cat Herder as a Model Context Protocol
//! server. See `PLAN.md` §14. A minimal, stateless JSON-RPC 2.0-over-HTTP server.

use cat_herder_application::start_run;
use cat_herder_domain::api::RunResponse;
use cat_herder_domain::entities::RunKind;
use cat_herder_domain::ports::{AgentRepository, RunRepository, RunRequest, SessionRepository};
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;

const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "cat-herder";
const SERVER_VERSION: &str = "0.1.0";

/// A JSON-RPC 2.0 request (MCP message).
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(default)]
    #[allow(dead_code)] // parsed but not yet validated against the spec version
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// Handle an MCP request and produce a JSON-RPC response (`Null` for notifications).
pub async fn handle(request: JsonRpcRequest, state: &AppState) -> Value {
    let id = request.id;
    // Notifications carry no id and expect no response.
    if id.is_none() {
        return Value::Null;
    }

    match request.method.as_str() {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
            },
        }),
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tools": tools() },
        }),
        "tools/call" => {
            let name = request.params["name"].as_str().unwrap_or("");
            let args = request.params.get("arguments").cloned().unwrap_or(Value::Null);
            let (text, is_error) = call_tool(name, &args, state).await;
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{ "type": "text", "text": text }],
                    "isError": is_error,
                },
            })
        }
        other => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("method not found: {other}") },
        }),
    }
}

fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "run_agent",
            "description": "Run an agent on a task — creates a sandbox and runs the harness.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "task_spec": { "type": "string", "description": "The task for the agent" },
                    "kind": { "type": "string", "enum": ["agent", "workflow"] },
                    "max_concurrency": { "type": "integer" },
                    "actor": { "type": "string", "description": "Who triggered the run" },
                    "model": { "type": "string", "description": "Agent model" },
                    "agent_id": { "type": "string", "description": "Agent template to use (its model applies)" },
                    "repo": { "type": "string", "description": "Repository" },
                    "language": { "type": "string", "description": "Primary coding language" }
                },
                "required": ["task_spec"],
            },
        }),
        json!({
            "name": "list_runs",
            "description": "List runs and their current lifecycle state.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": "list_sessions",
            "description": "List sessions (grouped runs) and their status.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": "list_agents",
            "description": "List available agent templates.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
    ]
}

async fn call_tool(name: &str, args: &Value, state: &AppState) -> (String, bool) {
    match name {
        "run_agent" => {
            let task_spec = args["task_spec"].as_str().unwrap_or("").to_string();
            if task_spec.is_empty() {
                return ("missing task_spec".into(), true);
            }
            let kind = match args["kind"].as_str().unwrap_or("agent") {
                "workflow" => RunKind::Workflow,
                _ => RunKind::Agent,
            };
            let max_concurrency = args["max_concurrency"].as_u64().unwrap_or(1) as u32;
            let opt = |k: &str| args[k].as_str().map(str::to_string);

            let Some(runtime) = &state.runtime else {
                return ("execution runtime not configured".into(), true);
            };

            // Models are agent-specific: resolve from agent_id or the explicit model.
            let model = match super::resolve_model(state, opt("model"), opt("agent_id")).await {
                Ok(Some(m)) => m,
                Ok(None) => return ("agent_id or model is required".into(), true),
                Err((_, msg)) => return (msg, true),
            };

            let req = RunRequest {
                kind,
                task_spec,
                dedup_key: String::new(),
                max_concurrency,
                trigger: "mcp".into(),
                actor: opt("actor"),
                model: Some(model),
                repo: opt("repo"),
                language: opt("language"),
            };

            match start_run(req, runtime, &state.repo, &state.sessions, &state.transitions, &state.bus).await {
                Ok(run) => (
                    serde_json::to_string(&RunResponse::from(&run)).unwrap_or_default(),
                    false,
                ),
                Err(e) => (format!("run failed: {e}"), true),
            }
        }
        "list_runs" => match state.repo.list_all().await {
            Ok(runs) => {
                let out: Vec<RunResponse> = runs.iter().map(RunResponse::from).collect();
                (serde_json::to_string(&out).unwrap_or_default(), false)
            }
            Err(e) => (format!("error: {e}"), true),
        },
        "list_sessions" => match state.sessions.list().await {
            Ok(sessions) => (serde_json::to_string(&sessions).unwrap_or_default(), false),
            Err(e) => (format!("error: {e}"), true),
        },
        "list_agents" => match state.agents.list().await {
            Ok(agents) => (serde_json::to_string(&agents).unwrap_or_default(), false),
            Err(e) => (format!("error: {e}"), true),
        },
        other => (format!("unknown tool: {other}"), true),
    }
}

//! MCP server (Streamable HTTP): expose Cat Herder as a Model Context Protocol
//! server. See `PLAN.md` §14. A minimal, stateless JSON-RPC 2.0-over-HTTP server.

use cat_herder_application::{respond_approval, start_run};
use cat_herder_domain::api::RunResponse;
use cat_herder_domain::entities::{ApprovalRequest, ApprovalStatus, RunKind};
use cat_herder_domain::ports::{
    AgentRepository, ApprovalRepository, RunRepository, RunRequest, SessionRepository,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;

const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "cat-herder";
const SERVER_VERSION: &str = "0.1.0";

/// A JSON-RPC 2.0 request (MCP message). The `method` field is empty for a *response*
/// to a server-initiated request (elicitation), which instead carries `result`/`error`.
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(default)]
    #[allow(dead_code)] // parsed but not yet validated against the spec version
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    #[allow(dead_code)]
    pub error: Option<Value>,
}

/// Handle an MCP request and produce a JSON-RPC response (`Null` for notifications).
///
/// `session_id` is the client's Streamable HTTP session (from the `mcp-session-id`
/// request header), if any. It is ignored for `initialize` (which always mints a fresh
/// session id) but is used to associate a run created by `run_agent` with that session
/// so terminal notifications reach its SSE stream.
pub async fn handle(request: JsonRpcRequest, state: &AppState, session_id: Option<&str>) -> Value {
    // A JSON-RPC *response* (to a server-initiated elicitation request) has no `method`
    // and carries a `result`. Route it to the elicitation response handler.
    if request.method.is_empty() && request.id.is_some() && request.result.is_some() {
        return handle_elicitation_response(request, state).await;
    }

    let id = request.id;
    // Notifications carry no id and expect no response.
    if id.is_none() {
        return Value::Null;
    }

    match request.method.as_str() {
        "initialize" => {
            let session_id = uuid::Uuid::new_v4().to_string();
            // Register the session's outbound channel up front so the client can open
            // its SSE stream immediately (before any run exists for it).
            state.mcp_hub.register_session(session_id.clone());
            // Remember whether this client advertises elicitation support, so an approval
            // can ask the triggerer directly when it can.
            let elicitation = request.params["capabilities"]["elicitation"].is_object();
            state.mcp_hub.set_elicit(&session_id, elicitation);
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
                    // Streamable HTTP session id: the client echoes it in the
                    // `mcp-session-id` header and opens the SSE stream with it.
                    "sessionId": session_id,
                },
            })
        }
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tools": tools() },
        }),
        "tools/call" => {
            let name = request.params["name"].as_str().unwrap_or("");
            let args = request
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Null);
            let (text, is_error) = call_tool(name, &args, state, session_id).await;
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
                    "language": { "type": "string", "description": "Primary coding language" },
                    "callback_url": { "type": "string", "description": "Optional https URL to POST the terminal result to" }
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
        json!({
            "name": "list_pending_approvals",
            "description": "List pending human-in-the-loop approval requests (awaiting approve/reject).",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": "respond_approval",
            "description": "Approve or reject a pending approval request (resumes or cancels the run).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "approval_id": { "type": "string", "description": "The approval request id" },
                    "approve": { "type": "boolean", "description": "true to approve/resume, false to reject/cancel" },
                    "response": { "type": "string", "description": "Optional note to send back to the agent" }
                },
                "required": ["approval_id", "approve"],
            },
        }),
    ]
}

async fn call_tool(
    name: &str,
    args: &Value,
    state: &AppState,
    session_id: Option<&str>,
) -> (String, bool) {
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

            // The per-agent harness image (falls back to the global image when absent).
            let agent_id = opt("agent_id");
            let image = match super::resolve_image(state, agent_id.as_deref()).await {
                Ok(img) => img,
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
                image,
                repo: opt("repo"),
                language: opt("language"),
                workflow_ref: None,
                runtime_tier: Default::default(),
                budget: args["budget"].as_u64(),
                callback_url: opt("callback_url"),
            };

            let budget = req.budget;
            match start_run(
                req,
                runtime,
                &state.repo,
                &state.sessions,
                &state.transitions,
                &state.bus,
                &state.broker,
                &state.run_status_notifier,
            )
            .await
            {
                Ok(run) => {
                    // Associate the new run with this MCP session so its subsequent
                    // lifecycle transitions are pushed to the client's SSE stream (if it
                    // opened one). Routed through the notifier backend (local or Redis).
                    if let Some(session_id) = session_id {
                        state
                            .run_status_notifier
                            .associate(run.id.0, session_id)
                            .await;
                    }
                    if let Err(e) =
                        super::issue_budget(&state.budgets, &state.budget_counter, run.id, budget)
                            .await
                    {
                        eprintln!("budget lease failed for run {}: {e}", run.id.0);
                    }
                    (
                        serde_json::to_string(&RunResponse::from(&run)).unwrap_or_default(),
                        false,
                    )
                }
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
        "list_pending_approvals" => match state.approvals.list_pending().await {
            Ok(approvals) => {
                let out: Vec<Value> = approvals
                    .iter()
                    .map(|a| {
                        json!({
                            "approval_id": a.id,
                            "run_id": a.run_id.0,
                            "payload": a.payload,
                        })
                    })
                    .collect();
                (serde_json::to_string(&out).unwrap_or_default(), false)
            }
            Err(e) => (format!("error: {e}"), true),
        },
        "respond_approval" => {
            let approval_id = args["approval_id"].as_str().unwrap_or("").to_string();
            if approval_id.is_empty() {
                return ("missing approval_id".into(), true);
            }
            let approve = args["approve"].as_bool().unwrap_or(false);
            let response = args["response"].as_str().map(str::to_string);

            // Load the approval to find its run and check its status.
            let approval = match state.approvals.get(&approval_id).await {
                Ok(Some(a)) => a,
                Ok(None) => return ("approval not found".into(), true),
                Err(e) => return (format!("error: {e}"), true),
            };
            if approval.status != ApprovalStatus::Pending {
                return ("approval already decided".into(), true);
            }

            // Scope: only the MCP session that triggered the run may respond to its
            // approval. A run with no associated session (e.g. created via REST) must be
            // answered through the web UI instead.
            let owner = state
                .run_status_notifier
                .session_for_run(approval.run_id.0)
                .await;
            match (session_id, owner) {
                (Some(caller), Some(owner)) if caller == owner => {}
                _ => {
                    return (
                        "not authorized to respond to this approval (triggered by another session)"
                            .into(),
                        true,
                    );
                }
            }

            let decided = match respond_approval(
                &approval_id,
                approve,
                response.clone(),
                &state.approvals,
                &state.events,
                &state.repo,
                &state.sessions,
                &state.transitions,
                &state.bus,
                &state.broker,
                &state.run_status_notifier,
            )
            .await
            {
                Ok(a) => a,
                Err(e) => return (format!("error: {e}"), true),
            };

            // Signal the live harness to resume (or cancel). No-op if the link is gone —
            // the run is already persisted and the reconciler re-creates the pod.
            let (signal, resp) = if decided.status == ApprovalStatus::Approved {
                ("resume", decided.response.clone())
            } else {
                ("cancel", None)
            };
            state
                .control_links
                .send_signal(decided.run_id, signal, resp)
                .await;

            (
                json!({
                    "id": decided.id,
                    "run_id": decided.run_id.0,
                    "status": decided.status,
                    "response": decided.response,
                })
                .to_string(),
                false,
            )
        }
        other => (format!("unknown tool: {other}"), true),
    }
}

/// If the run's trigger session advertised MCP elicitation support, send an
/// `elicitation/create` request over its SSE stream asking the triggerer to approve or
/// reject the pending approval. The elicitation registry is process-local (single
/// replica); multi-replica delivery relies on the plain notification above.
pub async fn elicit_approval(state: &AppState, approval: &ApprovalRequest) {
    let Some(session_id) = state
        .run_status_notifier
        .session_for_run(approval.run_id.0)
        .await
    else {
        return;
    };
    if !state.mcp_hub.supports_elicit(&session_id) {
        return;
    }

    let request_id = uuid::Uuid::new_v4().to_string();
    state
        .mcp_hub
        .register_elicitation(&request_id, approval.run_id.0, &approval.id);

    let message = format!("Run {} paused for approval.", approval.run_id.0);
    let msg = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "elicitation/create",
        "params": {
            "message": message,
            "requestedSchema": {
                "type": "object",
                "properties": {
                    "approve": { "type": "boolean", "description": "Approve (true) or reject (false) the pending action" },
                    "note": { "type": "string", "description": "Optional note to send back to the agent" }
                },
                "required": ["approve"]
            }
        }
    })
    .to_string();
    state.mcp_hub.dispatch(&session_id, msg);
}

/// Handle a JSON-RPC *response* to a server-initiated `elicitation/create` request: map
/// the triggerer's decision to an approve/reject and resume (or cancel) the run.
async fn handle_elicitation_response(request: JsonRpcRequest, state: &AppState) -> Value {
    let Some(request_id) = request
        .id
        .as_ref()
        .and_then(|v| v.as_str())
        .map(str::to_string)
    else {
        return Value::Null;
    };

    // Correlate with the pending elicitation (a unique nonce). Unknown/stale -> drop.
    let Some((run_id, approval_id)) = state.mcp_hub.take_elicitation(&request_id) else {
        return Value::Null;
    };

    let result = request.result.unwrap_or(Value::Null);
    let action = result.get("action").and_then(|a| a.as_str()).unwrap_or("");
    let content = result.get("content").cloned().unwrap_or(Value::Null);

    let (approved, response) = match action {
        "accept" => {
            let approve = content
                .get("approve")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let note = content
                .get("note")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            (approve, note)
        }
        // `decline`/`cancel` leave the approval pending (the web UI can still answer).
        _ => {
            eprintln!("elicitation {action} for run {run_id}; leaving approval pending");
            return Value::Null;
        }
    };

    let decided = match respond_approval(
        &approval_id,
        approved,
        response.clone(),
        &state.approvals,
        &state.events,
        &state.repo,
        &state.sessions,
        &state.transitions,
        &state.bus,
        &state.broker,
        &state.run_status_notifier,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            eprintln!("elicitation respond failed for run {run_id}: {e}");
            return Value::Null;
        }
    };

    let (signal, resp) = if decided.status == ApprovalStatus::Approved {
        ("resume", decided.response.clone())
    } else {
        ("cancel", None)
    };
    state
        .control_links
        .send_signal(decided.run_id, signal, resp)
        .await;

    // A response to a response is not a JSON-RPC concept — nothing to send back.
    Value::Null
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_request() -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "initialize".into(),
            params: Value::Null,
            result: None,
            error: None,
        }
    }

    #[tokio::test]
    async fn initialize_returns_a_session_id() {
        let state = AppState::default();
        let resp = handle(init_request(), &state, None).await;
        let session = resp["result"]["sessionId"]
            .as_str()
            .expect("sessionId present");
        assert!(!session.is_empty(), "session id must not be empty");
        // Protocol metadata is preserved for backward compatibility.
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(resp["result"]["serverInfo"]["name"], SERVER_NAME);
    }

    #[tokio::test]
    async fn initialize_generates_distinct_session_ids() {
        let state = AppState::default();
        let a = handle(init_request(), &state, None).await;
        let b = handle(init_request(), &state, None).await;
        assert_ne!(a["result"]["sessionId"], b["result"]["sessionId"]);
    }

    #[tokio::test]
    async fn notification_without_id_returns_null() {
        let state = AppState::default();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: None,
            method: "notifications/initialized".into(),
            params: Value::Null,
            result: None,
            error: None,
        };
        assert_eq!(handle(req, &state, None).await, Value::Null);
    }

    #[tokio::test]
    async fn initialize_records_elicitation_capability() {
        let state = AppState::default();
        let mut req = init_request();
        req.params = json!({"capabilities": {"elicitation": {}}});
        let resp = handle(req, &state, None).await;
        let session = resp["result"]["sessionId"].as_str().unwrap().to_string();
        assert!(state.mcp_hub.supports_elicit(&session));
    }

    #[tokio::test]
    async fn initialize_without_elicitation_sets_false() {
        let state = AppState::default();
        let resp = handle(init_request(), &state, None).await;
        let session = resp["result"]["sessionId"].as_str().unwrap().to_string();
        assert!(!state.mcp_hub.supports_elicit(&session));
    }

    #[tokio::test]
    async fn unknown_elicitation_response_is_ignored() {
        let state = AppState::default();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!("no-such-req")),
            method: String::new(),
            params: Value::Null,
            result: Some(json!({"action": "accept", "content": {"approve": true}})),
            error: None,
        };
        // An unmatched response is dropped without panicking or erroring.
        assert_eq!(handle(req, &state, None).await, Value::Null);
    }
}

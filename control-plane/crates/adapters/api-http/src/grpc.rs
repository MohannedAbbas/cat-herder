//! gRPC control channel (tonic): the harness (inside the sandbox pod) streams events,
//! heartbeats, and LangGraph checkpoints to the control plane (`PLAN.md` §6/§9/§14).
//! Replaces the HTTP `POST /v1/runs/{id}/events` path for the harness.
//!
//! The harness is the gRPC *client* — the untrusted pod reaches out to the trusted
//! control plane, matching the egress default-deny policy.

use std::collections::HashMap;
use std::time::SystemTime;

use tokio::sync::{Mutex, mpsc};
use tonic::codegen::BoxStream;
use tonic::{Request, Response, Status, Streaming};

use cat_herder_application::interrupt_run;
use cat_herder_domain::entities::{Checkpoint, RunId, Usage};
use cat_herder_domain::ports::CheckpointRepository;

use crate::pr::FileChange;
use crate::{AppState, ingest_harness_event};

/// Generated protobuf types (from `proto/control.proto`).
pub mod control {
    pub mod v1 {
        tonic::include_proto!("cat_herder.v1");
    }
}

use control::v1::harness_control_server::{HarnessControl, HarnessControlServer};
use control::v1::{
    Checkpoint as ProtoCheckpoint, ControlMessage, HarnessMessage, harness_message::Msg,
};

/// Build the tonic service for the harness control channel.
pub fn grpc_server(state: AppState) -> HarnessControlServer<HarnessControlService> {
    HarnessControlServer::new(HarnessControlService { state })
}

/// Registry of live harness control links: `run_id -> outbound message sender`. The
/// `connect` handler registers a run's sender after `Register`; the approval response
/// path sends `Signal` messages over it (`PLAN.md` §10). When a harness disconnects its
/// link is dropped.
#[derive(Default)]
pub struct ControlLinks {
    links: Mutex<HashMap<RunId, mpsc::Sender<Result<ControlMessage, Status>>>>,
}

impl ControlLinks {
    async fn register(&self, run_id: RunId, tx: mpsc::Sender<Result<ControlMessage, Status>>) {
        self.links.lock().await.insert(run_id, tx);
    }

    async fn unregister(&self, run_id: RunId) {
        self.links.lock().await.remove(&run_id);
    }

    /// Send a `Signal` (resume/cancel) to a run's harness, if its link is still open.
    /// Returns `false` when the run has no live link (e.g. the pod is gone).
    pub async fn send_signal(&self, run_id: RunId, signal: &str, response: Option<String>) -> bool {
        let msg = ControlMessage {
            msg: Some(control::v1::control_message::Msg::Signal(
                control::v1::Signal {
                    signal: signal.to_string(),
                    response: response.unwrap_or_default(),
                },
            )),
        };
        match self.links.lock().await.get(&run_id) {
            Some(tx) => tx.send(Ok(msg)).await.is_ok(),
            None => false,
        }
    }
}

/// Serve the harness control channel on `addr` (blocks until the server stops).
pub async fn grpc_serve(
    state: AppState,
    addr: std::net::SocketAddr,
) -> Result<(), tonic::transport::Error> {
    tonic::transport::Server::builder()
        .add_service(grpc_server(state))
        .serve(addr)
        .await
}

/// The `HarnessControl` service: ingests harness events and checkpoints over the
/// control channel, and hands back the latest checkpoint as the resume seed.
#[derive(Clone)]
pub struct HarnessControlService {
    state: AppState,
}

#[tonic::async_trait]
impl HarnessControl for HarnessControlService {
    type ConnectStream = BoxStream<ControlMessage>;

    async fn connect(
        &self,
        request: Request<Streaming<HarnessMessage>>,
    ) -> Result<Response<BoxStream<ControlMessage>>, Status> {
        let mut incoming = request.into_inner();

        // The first message must identify the run this channel belongs to.
        let run_id = match incoming.message().await? {
            Some(msg) => match msg.msg {
                Some(Msg::Register(r)) => r.run_id,
                _ => return Err(Status::invalid_argument("first message must be Register")),
            },
            None => return Err(Status::invalid_argument("stream closed before Register")),
        };
        let run_id = RunId(run_id);
        let state = self.state.clone();

        // The resume seed (latest checkpoint, or an empty one for a fresh run) is sent
        // on the response side before anything else, so the harness can seed its
        // checkpointer before running the agent (§6). Later signals (HITL, §10) will
        // reuse this same channel.
        let (tx, rx) = mpsc::channel::<Result<ControlMessage, Status>>(16);
        let seed = resume_seed(&state, run_id).await;
        if let Err(e) = tx.send(Ok(seed)).await {
            eprintln!(
                "control channel closed before resume seed for run {}: {e}",
                run_id.0
            );
        }

        // Register this run's outbound link so the approval path (§10) can signal it.
        state.control_links.register(run_id, tx.clone()).await;

        // Drain the harness's events/checkpoints/interrupts/heartbeats off the request
        // thread so the stream stays responsive.
        let ingest_state = state.clone();
        tokio::spawn(async move {
            loop {
                let msg = match incoming.message().await {
                    Ok(Some(m)) => m,
                    Ok(None) => break, // harness closed the stream (run done / pod gone)
                    Err(e) => {
                        eprintln!("gRPC control channel error for run {}: {e}", run_id.0);
                        break;
                    }
                };

                match msg.msg {
                    Some(Msg::Event(event)) => {
                        let changes = if event.changes.is_empty() {
                            None
                        } else {
                            Some(event.changes.into_iter().map(to_file_change).collect())
                        };
                        let usage = event.usage.map(to_usage);
                        let outcome = ingest_harness_event(
                            &ingest_state,
                            run_id,
                            &event.event_type,
                            empty_to_none(event.status),
                            empty_to_none(event.report),
                            changes,
                            usage,
                        )
                        .await;
                        if let Err((code, msg)) = outcome {
                            eprintln!("event ingest failed for run {}: {code}: {msg}", run_id.0);
                        }
                    }
                    Some(Msg::Checkpoint(cp)) => {
                        ingest_checkpoint(&ingest_state, run_id, cp).await;
                    }
                    Some(Msg::Interrupt(interrupt)) => {
                        ingest_interrupt(&ingest_state, run_id, interrupt).await;
                    }
                    // The stuck-run watchdog (a later step) consumes heartbeats; here we
                    // just keep the channel alive and acknowledge implicitly.
                    Some(Msg::Heartbeat(_)) => {}
                    _ => {}
                }
            }
            // The harness disconnected: drop the outbound link so late signals don't
            // block on a dead channel.
            ingest_state.control_links.unregister(run_id).await;
        });

        // Server -> harness direction: the resume seed now; signals (pause/resume/
        // cancel) once human-in-the-loop is wired (§10). The stream stays open for as
        // long as the harness keeps its side open.
        let out: BoxStream<ControlMessage> =
            Box::pin(futures::stream::unfold(rx, |mut rx| async move {
                rx.recv().await.map(|msg| (msg, rx))
            }));
        Ok(Response::new(out))
    }
}

/// Persist a harness-reported interrupt: create the durable approval and advance the
/// run to `AwaitingInput` (`PLAN.md` §10). Best-effort — errors are logged, not fatal.
async fn ingest_interrupt(state: &AppState, run_id: RunId, interrupt: control::v1::Interrupt) {
    let payload: serde_json::Value =
        serde_json::from_str(&interrupt.payload).unwrap_or(serde_json::Value::Null);
    let result = interrupt_run(
        run_id,
        interrupt.resume_token,
        payload,
        &state.approvals,
        &state.events,
        &state.repo,
        &state.sessions,
        &state.transitions,
        &state.bus,
    )
    .await;
    if let Err(e) = result {
        eprintln!("interrupt persist failed for run {}: {e}", run_id.0);
    }
}

/// Persist a harness-flushed LangGraph checkpoint (`PLAN.md` §6). Best-effort.
async fn ingest_checkpoint(state: &AppState, run_id: RunId, cp: control::v1::Checkpoint) {
    let checkpoint = Checkpoint {
        run_id,
        seq: 0,
        thread_id: cp.thread_id,
        checkpoint_ns: cp.checkpoint_ns,
        checkpoint_id: cp.checkpoint_id,
        serde_type: cp.serde_type,
        payload: cp.payload,
        occurred_at: SystemTime::now(),
    };
    if let Err(e) = state.checkpoints.append(run_id, &checkpoint).await {
        eprintln!("checkpoint persist failed for run {}: {e}", run_id.0);
    }
}

/// Build the resume seed: the run's latest checkpoint, or an empty checkpoint (fresh
/// run). Sent once after `Register`.
async fn resume_seed(state: &AppState, run_id: RunId) -> ControlMessage {
    let checkpoint = match state.checkpoints.latest(run_id).await {
        Ok(Some(cp)) => cp,
        _ => Checkpoint {
            run_id,
            seq: 0,
            thread_id: String::new(),
            checkpoint_ns: String::new(),
            checkpoint_id: String::new(),
            serde_type: String::new(),
            payload: Vec::new(),
            occurred_at: SystemTime::now(),
        },
    };
    ControlMessage {
        msg: Some(control::v1::control_message::Msg::Checkpoint(
            to_proto_checkpoint(&checkpoint),
        )),
    }
}

fn to_proto_checkpoint(cp: &Checkpoint) -> ProtoCheckpoint {
    ProtoCheckpoint {
        thread_id: cp.thread_id.clone(),
        checkpoint_ns: cp.checkpoint_ns.clone(),
        checkpoint_id: cp.checkpoint_id.clone(),
        serde_type: cp.serde_type.clone(),
        payload: cp.payload.clone(),
    }
}

fn empty_to_none(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

fn to_file_change(fc: control::v1::FileChange) -> FileChange {
    FileChange {
        path: fc.path,
        status: fc.status,
        content_base64: empty_to_none(fc.content_base64),
    }
}

fn to_usage(u: control::v1::Usage) -> Usage {
    Usage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CheckpointRepo;
    use cat_herder_store_inmemory::InMemoryCheckpointRepository;

    #[tokio::test]
    async fn resume_seed_is_empty_for_fresh_run() {
        let state = AppState::default();
        let seed = resume_seed(&state, RunId(1)).await;
        match seed.msg.expect("seed has a message") {
            control::v1::control_message::Msg::Checkpoint(cp) => {
                assert!(cp.checkpoint_id.is_empty());
                assert!(cp.payload.is_empty());
            }
            _ => panic!("expected a checkpoint seed"),
        }
    }

    #[tokio::test]
    async fn resume_seed_returns_latest_checkpoint() {
        let mut state = AppState::default();
        state.checkpoints = CheckpointRepo::InMemory(InMemoryCheckpointRepository::default());
        let cp = Checkpoint {
            run_id: RunId(7),
            seq: 0,
            thread_id: "7".into(),
            checkpoint_ns: String::new(),
            checkpoint_id: "abc-123".into(),
            serde_type: "json".into(),
            payload: b"data".to_vec(),
            occurred_at: SystemTime::now(),
        };
        state.checkpoints.append(RunId(7), &cp).await.unwrap();

        let seed = resume_seed(&state, RunId(7)).await;
        match seed.msg.expect("seed has a message") {
            control::v1::control_message::Msg::Checkpoint(c) => {
                assert_eq!(c.checkpoint_id, "abc-123");
                assert_eq!(c.thread_id, "7");
                assert_eq!(c.payload, b"data");
                assert_eq!(c.serde_type, "json");
            }
            _ => panic!("expected a checkpoint seed"),
        }
    }

    #[tokio::test]
    async fn control_links_routes_signal_and_unregisters() {
        let links = ControlLinks::default();
        let (tx, mut rx) = mpsc::channel::<Result<ControlMessage, Status>>(8);
        links.register(RunId(7), tx).await;

        // A registered run receives the signal.
        assert!(
            links
                .send_signal(RunId(7), "resume", Some("ok".into()))
                .await
        );
        let msg = rx.recv().await.expect("signal sent").expect("ok");
        match msg.msg.expect("has a message") {
            control::v1::control_message::Msg::Signal(s) => {
                assert_eq!(s.signal, "resume");
                assert_eq!(s.response, "ok");
            }
            _ => panic!("expected a signal"),
        }

        // An unregistered run has no live link.
        assert!(!links.send_signal(RunId(8), "resume", None).await);

        // After unregister, the link is gone.
        links.unregister(RunId(7)).await;
        assert!(!links.send_signal(RunId(7), "resume", None).await);
    }

    #[tokio::test]
    async fn ingest_interrupt_creates_approval_and_awaits_input() {
        use cat_herder_domain::entities::{Run, RunKind, RunState};
        use cat_herder_domain::ports::{EventRepository, RunRepository};

        let state = AppState::default();
        let run = Run {
            id: RunId(42),
            kind: RunKind::Agent,
            state: RunState::Running,
            task_spec: "interrupt me".into(),
            session_id: None,
            max_concurrency: 1,
            created_at: SystemTime::now(),
            trigger: "api".into(),
            actor: None,
            model: None,
            repo: None,
            language: None,
            started_at: None,
            finished_at: None,
            pr_url: None,
            report: None,
            input_tokens: 0,
            output_tokens: 0,
        };
        state.repo.save(&run).await.unwrap();

        ingest_interrupt(
            &state,
            RunId(42),
            control::v1::Interrupt {
                resume_token: "ckpt-1".into(),
                payload: r#"{"tool":"push"}"#.into(),
            },
        )
        .await;

        // The run paused for input, and the approval + its event were recorded.
        assert_eq!(
            state.repo.get(RunId(42)).await.unwrap().unwrap().state,
            RunState::AwaitingInput
        );
        let types: Vec<String> = state
            .events
            .list(RunId(42), 0, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.event_type)
            .collect();
        assert!(types.contains(&"approval.requested".to_string()));
        assert!(types.contains(&"run.awaiting_input".to_string()));
    }

    #[tokio::test]
    async fn ingest_checkpoint_persists_checkpoint() {
        use cat_herder_domain::ports::CheckpointRepository;

        let state = AppState::default();
        ingest_checkpoint(
            &state,
            RunId(9),
            control::v1::Checkpoint {
                thread_id: "9".into(),
                checkpoint_ns: String::new(),
                checkpoint_id: "c9".into(),
                serde_type: "json".into(),
                payload: b"data".to_vec(),
            },
        )
        .await;

        let latest = state.checkpoints.latest(RunId(9)).await.unwrap().unwrap();
        assert_eq!(latest.checkpoint_id, "c9");
        assert_eq!(latest.payload, b"data");
    }

    #[test]
    fn proto_conversion_helpers() {
        assert_eq!(empty_to_none(String::new()), None);
        assert_eq!(empty_to_none("x".into()), Some("x".into()));

        let fc = to_file_change(control::v1::FileChange {
            path: "a".into(),
            status: "modified".into(),
            content_base64: String::new(),
        });
        assert_eq!(fc.path, "a");
        assert_eq!(fc.content_base64, None);

        let usage = to_usage(control::v1::Usage {
            input_tokens: 3,
            output_tokens: 4,
        });
        assert_eq!((usage.input_tokens, usage.output_tokens), (3, 4));

        let proto = to_proto_checkpoint(&Checkpoint {
            run_id: RunId(1),
            seq: 0,
            thread_id: "1".into(),
            checkpoint_ns: String::new(),
            checkpoint_id: "c".into(),
            serde_type: "json".into(),
            payload: b"p".to_vec(),
            occurred_at: SystemTime::now(),
        });
        assert_eq!(proto.checkpoint_id, "c");
        assert_eq!(proto.payload, b"p");
    }
}

//! gRPC control channel (tonic): the harness (inside the sandbox pod) streams events
//! and heartbeats to the control plane (`PLAN.md` §6/§9/§14). Replaces the HTTP
//! `POST /v1/runs/{id}/events` path for the harness.
//!
//! The harness is the gRPC *client* — the untrusted pod reaches out to the trusted
//! control plane, matching the egress default-deny policy.

use tonic::codegen::BoxStream;
use tonic::{Request, Response, Status, Streaming};

use cat_herder_domain::entities::{RunId, Usage};

use crate::pr::FileChange;
use crate::{AppState, ingest_harness_event};

/// Generated protobuf types (from `proto/control.proto`).
pub mod control {
    pub mod v1 {
        tonic::include_proto!("cat_herder.v1");
    }
}

use control::v1::harness_control_server::{HarnessControl, HarnessControlServer};
use control::v1::{ControlMessage, HarnessMessage, harness_message::Msg};

/// Build the tonic service for the harness control channel.
pub fn grpc_server(state: AppState) -> HarnessControlServer<HarnessControlService> {
    HarnessControlServer::new(HarnessControlService { state })
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

/// The `HarnessControl` service: ingests harness events over the control channel.
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

        // Drain the harness's events/heartbeats off the request thread so the stream
        // stays responsive. The response (server -> harness) side stays open but empty
        // until human-in-the-loop signals are wired (PLAN.md §10).
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
                            &state,
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
                    // The stuck-run watchdog (a later step) consumes heartbeats; here we
                    // just keep the channel alive and acknowledge implicitly.
                    Some(Msg::Heartbeat(_)) => {}
                    _ => {}
                }
            }
        });

        // Server -> harness direction: empty until signals (pause/resume/cancel) are
        // wired with human-in-the-loop. `pending()` never yields or completes, so the
        // RPC stays open for as long as the harness keeps its side open.
        let out: BoxStream<ControlMessage> = Box::pin(futures::stream::pending());
        Ok(Response::new(out))
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

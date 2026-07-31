//! Loopback-only HTTP/WebSocket gateway for the bundled Remote Control PWA.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::OpenOptions;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::body::Body;
use axum::extract::ws::{close_code, CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Path as AxumPath, State};
use axum::http::header::{
    ACCEPT, CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, COOKIE, HOST, ORIGIN,
    REFERRER_POLICY, SET_COOKIE, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use clinch_companion_protocol::{
    decode_upload_chunk, encode_terminal_output, AuthChallengeRequest, Authenticate,
    ClientEnvelope, ClientMessage, PairingClaimRequest, PairingStatusRequest, ProtocolError,
    ProtocolErrorCode, RequestId, ServerEnvelope, ServerMessage, UploadProgress,
    AUTH_SESSION_TTL_SECS, MAX_JSON_MESSAGE_BYTES, MAX_TERMINAL_FRAME_BYTES,
    MAX_UPLOAD_CHUNK_BYTES, PROTOCOL_VERSION,
};
use instant::Instant;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::oneshot;
use warpui::ModelSpawner;

use super::pairing::{PairingError, PairingErrorKind, PairingManager};
use super::workspace_adapter::{
    TerminalOutputStream, UploadCompletion, UploadPlan, WorkspaceAdapter,
};

const SESSION_COOKIE_NAME: &str = "clinch_remote_session";
const MAX_CLIENT_MESSAGES_PER_SECOND: usize = 128;
const STATIC_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
    img-src 'self' data: blob:; font-src 'self'; connect-src 'self' wss:; object-src 'none'; \
    base-uri 'none'; frame-ancestors 'none'; form-action 'self'; manifest-src 'self'; worker-src 'self'";

#[derive(Clone, Debug)]
pub enum GatewayEvent {
    PendingPairingChanged,
    DeviceRegistryChanged,
    ClientConnected,
    ClientDisconnected,
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("the Remote Control web app is not built at {0}")]
    AssetsUnavailable(PathBuf),
    #[error("could not bind the Remote Control loopback listener: {0}")]
    Bind(std::io::Error),
    #[error("the Remote Control public origin is invalid")]
    InvalidOrigin,
}

#[derive(Clone)]
struct GatewayState {
    pairing: PairingManager,
    workspace_spawner: ModelSpawner<WorkspaceAdapter>,
    events: async_channel::Sender<GatewayEvent>,
    security: GatewaySecurity,
    assets_root: Arc<PathBuf>,
    route_path: Arc<String>,
}

#[derive(Clone, Default)]
pub struct GatewaySecurity {
    inner: Arc<RwLock<GatewaySecurityState>>,
}

#[derive(Default)]
struct GatewaySecurityState {
    expected_origin: Option<String>,
    accepted_hosts: HashSet<String>,
}

impl GatewaySecurity {
    fn with_loopback_host(host: String) -> Self {
        let mut accepted_hosts = HashSet::new();
        accepted_hosts.insert(host);
        Self {
            inner: Arc::new(RwLock::new(GatewaySecurityState {
                expected_origin: None,
                accepted_hosts,
            })),
        }
    }

    pub fn set_public_origin(&self, origin: &str) -> Result<(), GatewayError> {
        let parsed = url::Url::parse(origin).map_err(|_| GatewayError::InvalidOrigin)?;
        if parsed.scheme() != "https"
            || parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.port().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(GatewayError::InvalidOrigin);
        }
        let host = parsed.host_str().ok_or(GatewayError::InvalidOrigin)?;
        let expected_origin = format!("https://{host}");
        let mut state = self
            .inner
            .write()
            .map_err(|_| GatewayError::InvalidOrigin)?;
        state.expected_origin = Some(expected_origin);
        state.accepted_hosts.insert(host.to_owned());
        Ok(())
    }

    fn validate_host(&self, headers: &HeaderMap) -> bool {
        let Ok(state) = self.inner.read() else {
            return false;
        };
        headers
            .get(HOST)
            .and_then(|host| host.to_str().ok())
            .is_some_and(|host| state.accepted_hosts.contains(host))
    }

    fn validate_origin(&self, headers: &HeaderMap) -> bool {
        let Ok(state) = self.inner.read() else {
            return false;
        };
        let Some(expected) = &state.expected_origin else {
            return false;
        };
        headers
            .get(ORIGIN)
            .and_then(|origin| origin.to_str().ok())
            .is_some_and(|origin| origin == expected)
    }
}

pub struct GatewayHandle {
    pub port: u16,
    pub security: GatewaySecurity,
    shutdown: Option<oneshot::Sender<()>>,
}

impl GatewayHandle {
    pub fn start(
        runtime: &tokio::runtime::Runtime,
        route_path: String,
        pairing: PairingManager,
        workspace_spawner: ModelSpawner<WorkspaceAdapter>,
        events: async_channel::Sender<GatewayEvent>,
        assets_root: PathBuf,
    ) -> Result<Self, GatewayError> {
        if !assets_root.join("index.html").is_file() {
            return Err(GatewayError::AssetsUnavailable(assets_root));
        }
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                0,
            )))
            .map_err(GatewayError::Bind)?;
        let address = listener.local_addr().map_err(GatewayError::Bind)?;
        let security = GatewaySecurity::with_loopback_host(address.to_string());
        let state = GatewayState {
            pairing,
            workspace_spawner,
            events,
            security: security.clone(),
            assets_root: Arc::new(assets_root),
            route_path: Arc::new(route_path.clone()),
        };
        // Tailscale Serve uses `route_path` as its public mount point and strips that prefix
        // before proxying to this loopback listener. Keep the public path in cookies and pairing
        // URLs, but serve the backend at `/` so `/clinch-remote-…/api` reaches `/api` here.
        let router = route_for_state(state);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        runtime.spawn(async move {
            let server = axum::serve(listener, router).with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });
            if let Err(error) = server.await {
                log::warn!("Remote Control gateway stopped: {error}");
            }
        });

        Ok(Self {
            port: address.port(),
            security,
            shutdown: Some(shutdown_tx),
        })
    }
}

impl Drop for GatewayHandle {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

pub fn locate_assets() -> PathBuf {
    if cfg!(debug_assertions) {
        if let Some(path) = std::env::var_os("CLINCH_REMOTE_CONTROL_WEB_DIR") {
            return PathBuf::from(path);
        }
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("app crate has a workspace parent");
        let development = workspace.join("web/remote-control/dist");
        if development.join("index.html").is_file() {
            return development;
        }
    }
    warp_core::paths::bundled_resources_dir()
        .unwrap_or_default()
        .join("remote-control-web")
}

fn route_for_state(state: GatewayState) -> Router {
    Router::new()
        .route("/api/v1/pair/claim", post(pair_claim))
        .route("/api/v1/pair/status", post(pair_status))
        .route("/api/v1/auth/challenge", post(auth_challenge))
        .route("/api/v1/auth/authenticate", post(authenticate))
        .route("/ws", get(websocket))
        .route("/", get(static_index))
        .route("/{*asset}", get(static_asset))
        .layer(DefaultBodyLimit::max(MAX_JSON_MESSAGE_BYTES))
        .with_state(state)
}

async fn pair_claim(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<PairingClaimRequest>,
) -> Response {
    if !validate_api_headers(&state, &headers) {
        return http_error(
            StatusCode::FORBIDDEN,
            "request_origin",
            "Request origin was rejected.",
        );
    }
    match state.pairing.claim(request, Utc::now()) {
        Ok(receipt) => {
            let _ = state.events.send(GatewayEvent::PendingPairingChanged).await;
            api_json(StatusCode::CREATED, &receipt)
        }
        Err(error) => pairing_error(error),
    }
}

async fn pair_status(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<PairingStatusRequest>,
) -> Response {
    if !validate_api_headers(&state, &headers) {
        return http_error(
            StatusCode::FORBIDDEN,
            "request_origin",
            "Request origin was rejected.",
        );
    }
    match state.pairing.pairing_status(request, Utc::now()) {
        Ok(status) => api_json(StatusCode::OK, &status),
        Err(error) => pairing_error(error),
    }
}

async fn auth_challenge(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<AuthChallengeRequest>,
) -> Response {
    if !validate_api_headers(&state, &headers) {
        return http_error(
            StatusCode::FORBIDDEN,
            "request_origin",
            "Request origin was rejected.",
        );
    }
    match state.pairing.create_challenge(request, Utc::now()) {
        Ok(challenge) => api_json(StatusCode::OK, &challenge),
        Err(error) => pairing_error(error),
    }
}

async fn authenticate(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Json(request): Json<Authenticate>,
) -> Response {
    if !validate_api_headers(&state, &headers) {
        return http_error(
            StatusCode::FORBIDDEN,
            "request_origin",
            "Request origin was rejected.",
        );
    }
    match state.pairing.authenticate(request, Utc::now()) {
        Ok(session) => {
            let cookie = format!(
                "{SESSION_COOKIE_NAME}={}; Path={}/; Max-Age={AUTH_SESSION_TTL_SECS}; Secure; \
                 HttpOnly; SameSite=Strict",
                session.cookie_token,
                state.route_path.trim_end_matches('/')
            );
            let _ = state.events.send(GatewayEvent::DeviceRegistryChanged).await;
            let mut response = api_json(StatusCode::OK, &session.response);
            if let Ok(value) = HeaderValue::from_str(&cookie) {
                response.headers_mut().insert(SET_COOKIE, value);
            }
            response
        }
        Err(error) => pairing_error(error),
    }
}

async fn websocket(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !validate_api_headers(&state, &headers) {
        return http_error(
            StatusCode::FORBIDDEN,
            "request_origin",
            "Request origin was rejected.",
        );
    }
    let Some(cookie_token) = session_cookie(&headers) else {
        return http_error(
            StatusCode::UNAUTHORIZED,
            "authentication",
            "Pairing is required.",
        );
    };
    // Validate before accepting the upgrade, then atomically claim the cookie inside the upgraded
    // task. If the HTTP upgrade is abandoned, it therefore cannot consume a connection slot.
    if let Err(error) = state.pairing.authorize_session(&cookie_token, Utc::now()) {
        return pairing_error(error);
    }

    let max_websocket_message = MAX_JSON_MESSAGE_BYTES.max(MAX_UPLOAD_CHUNK_BYTES + 64);
    ws.max_message_size(max_websocket_message)
        .max_frame_size(max_websocket_message)
        .on_upgrade(move |socket| websocket_loop(socket, state, cookie_token))
}

struct ActiveUpload {
    plan: UploadPlan,
    staging_path: PathBuf,
    file: tokio::fs::File,
    next_chunk_index: u64,
    received: u64,
    digest: Sha256,
}

async fn websocket_loop(mut socket: WebSocket, state: GatewayState, cookie_token: String) {
    let initial_authorization = match state.pairing.connect_session(&cookie_token, Utc::now()) {
        Ok(authorization) => authorization,
        Err(_) => {
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: close_code::POLICY,
                    reason: "Authorization expired or was revoked".into(),
                })))
                .await;
            return;
        }
    };
    let session_id = initial_authorization.session_id;
    let _ = state.events.send(GatewayEvent::ClientConnected).await;
    let mut connection_sequence = 0u64;
    let hello = ServerEnvelope {
        version: PROTOCOL_VERSION,
        request_id: None,
        sequence: None,
        payload: ServerMessage::Hello {
            supported_versions: clinch_companion_protocol::SUPPORTED_PROTOCOL_VERSIONS.to_vec(),
            host_name: gethostname::gethostname().to_string_lossy().into_owned(),
        },
    };
    if send_json(&mut socket, &mut connection_sequence, &hello)
        .await
        .is_err()
    {
        let _ = state.pairing.end_session(session_id);
        let _ = state.events.send(GatewayEvent::ClientDisconnected).await;
        return;
    }
    let snapshot = match state
        .workspace_spawner
        .spawn(|adapter, ctx| adapter.initial_snapshot(ctx))
        .await
    {
        Ok(snapshot) => snapshot,
        Err(_) => {
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: close_code::ERROR,
                    reason: "Clinch workspace is unavailable".into(),
                })))
                .await;
            let _ = state.pairing.end_session(session_id);
            let _ = state.events.send(GatewayEvent::ClientDisconnected).await;
            return;
        }
    };
    if send_json(&mut socket, &mut connection_sequence, &snapshot)
        .await
        .is_err()
    {
        let _ = state.pairing.end_session(session_id);
        let _ = state.events.send(GatewayEvent::ClientDisconnected).await;
        return;
    }
    let mut last_workspace = snapshot_workspace_fingerprint(&snapshot);
    let mut snapshot_interval = tokio::time::interval(Duration::from_secs(1));
    snapshot_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    snapshot_interval.tick().await;
    let mut terminal_stream: Option<TerminalOutputStream> = None;
    let mut terminal_sequence = 0u64;
    let mut active_upload: Option<ActiveUpload> = None;
    let mut completed_upload_requests = HashMap::<RequestId, ServerEnvelope>::new();
    let mut message_times = VecDeque::<Instant>::new();

    'connection: loop {
        tokio::select! {
            message = socket.recv() => {
                let Some(message) = message else { break; };
                let authorization = match state.pairing.authorize_session(&cookie_token, Utc::now()) {
                    Ok(authorization) => authorization,
                    Err(_) => {
                        let _ = socket
                            .send(Message::Close(Some(CloseFrame {
                                code: close_code::POLICY,
                                reason: "Authorization expired or was revoked".into(),
                            })))
                            .await;
                        break;
                    }
                };
                if matches!(&message, Ok(Message::Text(_) | Message::Binary(_)))
                    && !admit_client_message(&mut message_times, Instant::now())
                {
                    let error = protocol_error(
                        None,
                        ProtocolErrorCode::RateLimited,
                        "This phone is sending commands too quickly.",
                    );
                    let _ = send_json(&mut socket, &mut connection_sequence, &error).await;
                    break;
                }
                match message {
            Ok(Message::Text(text)) => {
                let envelope = match serde_json::from_str::<ClientEnvelope>(&text) {
                    Ok(envelope) => envelope,
                    Err(_) => {
                        let error = protocol_error(
                            None,
                            ProtocolErrorCode::InvalidRequest,
                            "Invalid message.",
                        );
                        if send_json(&mut socket, &mut connection_sequence, &error).await.is_err() {
                            break;
                        }
                        continue;
                    }
                };
                if matches!(envelope.payload, ClientMessage::Disconnect) {
                    let _ = socket.send(Message::Close(None)).await;
                    break;
                }
                if let Some(cached) = completed_upload_requests.get(&envelope.request_id) {
                    if send_json(&mut socket, &mut connection_sequence, cached).await.is_err() {
                        break;
                    }
                    continue;
                }

                if let ClientMessage::UploadCancel(cancel) = &envelope.payload {
                    let cancelled = active_upload
                        .as_ref()
                        .is_some_and(|upload| upload.plan.upload_id == cancel.upload_id);
                    if cancelled {
                        if let Some(upload) = active_upload.take() {
                            cleanup_staging_file(&upload.staging_path);
                        }
                    }
                    let response = protocol_command_accepted(
                        Some(envelope.request_id),
                        if cancelled { "Upload cancelled." } else { "Upload was already inactive." },
                    );
                    if send_json(&mut socket, &mut connection_sequence, &response).await.is_err() {
                        break;
                    }
                    continue;
                }

                if let ClientMessage::UploadCommit(commit) = &envelope.payload {
                    let request_id = envelope.request_id;
                    let Some(upload) = active_upload.take() else {
                        let error = protocol_error(
                            Some(request_id),
                            ProtocolErrorCode::UploadRejected,
                            "No matching upload is active on this connection.",
                        );
                        if send_json(&mut socket, &mut connection_sequence, &error).await.is_err() { break; }
                        continue;
                    };
                    if upload.plan.upload_id != commit.upload_id {
                        cleanup_staging_file(&upload.staging_path);
                        let error = protocol_error(
                            Some(request_id),
                            ProtocolErrorCode::UploadRejected,
                            "The upload ID does not match the active upload.",
                        );
                        if send_json(&mut socket, &mut connection_sequence, &error).await.is_err() { break; }
                        continue;
                    }
                    match finalize_upload(upload).await {
                        Ok((plan, final_path)) => {
                            let final_path_string = final_path.to_string_lossy().into_owned();
                            let expected_directory = plan.destination_directory.clone();
                            let target = plan.target.clone();
                            let upload_id = plan.upload_id;
                            let response = state.workspace_spawner.spawn(move |adapter, ctx| {
                                adapter.complete_upload(
                                    UploadCompletion {
                                        request_id: Some(request_id),
                                        upload_id,
                                        target,
                                        expected_directory,
                                        final_path: final_path_string,
                                        authorization,
                                    },
                                    ctx,
                                )
                            }).await.unwrap_or_else(|_| protocol_error(
                                Some(request_id),
                                ProtocolErrorCode::Internal,
                                "Clinch workspace is unavailable.",
                            ));
                            if matches!(response.payload, ServerMessage::Error(_)) {
                                let _ = std::fs::remove_file(&final_path);
                            }
                            completed_upload_requests.insert(request_id, response.clone());
                            if completed_upload_requests.len() > 64 {
                                if let Some(oldest) = completed_upload_requests.keys().next().copied() {
                                    completed_upload_requests.remove(&oldest);
                                }
                            }
                            if send_json(&mut socket, &mut connection_sequence, &response).await.is_err() { break; }
                        }
                        Err(message) => {
                            let error = protocol_error(
                                Some(request_id),
                                ProtocolErrorCode::UploadRejected,
                                &message,
                            );
                            if send_json(&mut socket, &mut connection_sequence, &error).await.is_err() { break; }
                        }
                    }
                    continue;
                }

                if matches!(envelope.payload, ClientMessage::UploadBegin(_)) && active_upload.is_some() {
                    let error = protocol_error(
                        Some(envelope.request_id),
                        ProtocolErrorCode::UploadRejected,
                        "Finish or cancel the current upload first.",
                    );
                    if send_json(&mut socket, &mut connection_sequence, &error).await.is_err() { break; }
                    continue;
                }

                let reply = state
                    .workspace_spawner
                    .spawn(move |adapter, ctx| {
                        adapter.handle_envelope(envelope, authorization, ctx)
                    })
                    .await;
                let mut reply = match reply {
                    Ok(reply) => reply,
                    Err(_) => super::workspace_adapter::AdapterReply {
                        envelope: protocol_error(
                        None,
                        ProtocolErrorCode::Internal,
                        "Clinch workspace is unavailable.",
                        ),
                        terminal_stream: None,
                        upload_plan: None,
                    },
                };
                if let Some(plan) = reply.upload_plan.take() {
                    match stage_upload(plan) {
                        Ok(upload) => active_upload = Some(upload),
                        Err(message) => {
                            reply.envelope = protocol_error(
                                reply.envelope.request_id,
                                ProtocolErrorCode::UploadRejected,
                                &message,
                            );
                        }
                    }
                }
                if let Some(stream) = reply.terminal_stream.take() {
                    terminal_stream = Some(stream);
                    terminal_sequence = 0;
                }
                if send_json(&mut socket, &mut connection_sequence, &reply.envelope).await.is_err() {
                    break;
                }
            }
            Ok(Message::Binary(bytes)) => {
                let frame = match decode_upload_chunk(&bytes) {
                    Ok(frame) => frame,
                    Err(_) => {
                        if let Some(upload) = active_upload.take() {
                            cleanup_staging_file(&upload.staging_path);
                        }
                        let error = protocol_error(
                            None,
                            ProtocolErrorCode::UploadRejected,
                            "The upload chunk frame is invalid.",
                        );
                        if send_json(&mut socket, &mut connection_sequence, &error).await.is_err() { break; }
                        continue;
                    }
                };
                let Some(upload) = active_upload.as_mut() else {
                    let error = protocol_error(
                        None,
                        ProtocolErrorCode::UploadRejected,
                        "No upload is active for this connection.",
                    );
                    if send_json(&mut socket, &mut connection_sequence, &error).await.is_err() { break; }
                    continue;
                };
                if frame.upload_id != upload.plan.upload_id
                    || frame.chunk_index != upload.next_chunk_index
                    || upload.received.saturating_add(frame.payload.len() as u64) > upload.plan.size
                {
                    let upload = active_upload.take().expect("active upload was just checked");
                    cleanup_staging_file(&upload.staging_path);
                    let error = protocol_error(
                        None,
                        ProtocolErrorCode::UploadRejected,
                        "Upload chunks must arrive once, in order, and within the declared size.",
                    );
                    if send_json(&mut socket, &mut connection_sequence, &error).await.is_err() { break; }
                    continue;
                }
                if upload.file.write_all(frame.payload).await.is_err() {
                    let upload = active_upload.take().expect("active upload was just checked");
                    cleanup_staging_file(&upload.staging_path);
                    let error = protocol_error(
                        None,
                        ProtocolErrorCode::UploadRejected,
                        "The Mac could not write the uploaded file.",
                    );
                    if send_json(&mut socket, &mut connection_sequence, &error).await.is_err() { break; }
                    continue;
                }
                upload.digest.update(frame.payload);
                upload.received += frame.payload.len() as u64;
                upload.next_chunk_index += 1;
                let progress = ServerEnvelope {
                    version: PROTOCOL_VERSION,
                    request_id: None,
                    sequence: None,
                    payload: ServerMessage::UploadProgress(UploadProgress {
                        upload_id: upload.plan.upload_id,
                        received: upload.received,
                        total: upload.plan.size,
                    }),
                };
                if send_json(&mut socket, &mut connection_sequence, &progress).await.is_err() { break; }
            }
            Ok(Message::Ping(payload)) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Ok(Message::Pong(_)) => {}
            Ok(Message::Close(_)) | Err(_) => break,
                }
            }
            terminal = receive_terminal_output(&mut terminal_stream) => {
                match terminal {
                    Ok(bytes) => {
                        let Some(stream) = terminal_stream.as_ref() else { continue; };
                        for chunk in bytes.chunks(MAX_TERMINAL_FRAME_BYTES) {
                            terminal_sequence = terminal_sequence.saturating_add(1);
                            let Ok(frame) = encode_terminal_output(
                                stream.stream_id,
                                terminal_sequence,
                                chunk,
                            ) else {
                                continue;
                            };
                            if socket.send(Message::Binary(frame.into())).await.is_err() {
                                break 'connection;
                            }
                        }
                    }
                    Err(async_broadcast::RecvError::Overflowed(_)) => {
                        if let Some(stream) = terminal_stream.take() {
                            let closed = ServerEnvelope {
                                version: PROTOCOL_VERSION,
                                request_id: None,
                                sequence: None,
                                payload: ServerMessage::TerminalStreamClosed {
                                    stream_id: stream.stream_id,
                                    reason: "Terminal output moved too quickly; reselecting the terminal will resync it.".to_owned(),
                                },
                            };
                            if send_json(&mut socket, &mut connection_sequence, &closed).await.is_err() { break; }
                        }
                    }
                    Err(async_broadcast::RecvError::Closed) => {
                        if let Some(stream) = terminal_stream.take() {
                            let closed = ServerEnvelope {
                                version: PROTOCOL_VERSION,
                                request_id: None,
                                sequence: None,
                                payload: ServerMessage::TerminalStreamClosed {
                                    stream_id: stream.stream_id,
                                    reason: "The terminal closed on the Mac.".to_owned(),
                                },
                            };
                            if send_json(&mut socket, &mut connection_sequence, &closed).await.is_err() { break; }
                        }
                    }
                }
            }
            _ = snapshot_interval.tick() => {
                if state.pairing.authorize_session(&cookie_token, Utc::now()).is_err() {
                    let _ = socket.send(Message::Close(Some(CloseFrame {
                        code: close_code::POLICY,
                        reason: "Authorization expired or was revoked".into(),
                    }))).await;
                    break;
                }
                let polled = state.workspace_spawner
                    .spawn(|adapter, ctx| adapter.poll_snapshot(ctx))
                    .await;
                let Ok(snapshot) = polled else { continue; };
                let fingerprint = workspace_fingerprint(&snapshot);
                if last_workspace.as_ref() != Some(&fingerprint) {
                    last_workspace = Some(fingerprint);
                    let changed = state.workspace_spawner
                        .spawn(move |adapter, _| adapter.workspace_changed(snapshot))
                        .await;
                    let Ok(changed) = changed else { continue; };
                    if send_json(&mut socket, &mut connection_sequence, &changed).await.is_err() { break; }
                }
            }
        }
    }
    if let Some(upload) = active_upload.take() {
        cleanup_staging_file(&upload.staging_path);
    }
    let _ = state
        .workspace_spawner
        .spawn(move |adapter, ctx| adapter.session_disconnected(session_id, ctx))
        .await;
    let _ = state.pairing.end_session(session_id);
    let _ = state.events.send(GatewayEvent::ClientDisconnected).await;
}

fn admit_client_message(message_times: &mut VecDeque<Instant>, now: Instant) -> bool {
    let cutoff = now.checked_sub(Duration::from_secs(1)).unwrap_or(now);
    while message_times.front().is_some_and(|time| *time <= cutoff) {
        message_times.pop_front();
    }
    if message_times.len() >= MAX_CLIENT_MESSAGES_PER_SECOND {
        return false;
    }
    message_times.push_back(now);
    true
}

async fn receive_terminal_output(
    stream: &mut Option<TerminalOutputStream>,
) -> Result<Arc<Vec<u8>>, async_broadcast::RecvError> {
    match stream {
        Some(stream) => stream.receiver.recv().await,
        None => std::future::pending().await,
    }
}

fn stage_upload(plan: UploadPlan) -> Result<ActiveUpload, String> {
    let staging_path = plan
        .destination_directory
        .join(format!(".clinch-upload-{}.part", plan.upload_id));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(&staging_path).map_err(|error| {
        format!("Could not stage the upload in the terminal directory: {error}")
    })?;
    Ok(ActiveUpload {
        plan,
        staging_path,
        file: tokio::fs::File::from_std(file),
        next_chunk_index: 0,
        received: 0,
        digest: Sha256::new(),
    })
}

async fn finalize_upload(mut upload: ActiveUpload) -> Result<(UploadPlan, PathBuf), String> {
    if upload.received != upload.plan.size {
        cleanup_staging_file(&upload.staging_path);
        return Err("The upload ended before the declared file size was received.".to_owned());
    }
    if let Err(error) = upload.file.flush().await {
        drop(upload.file);
        cleanup_staging_file(&upload.staging_path);
        return Err(format!("Could not flush the uploaded file: {error}"));
    }
    if let Err(error) = upload.file.sync_all().await {
        drop(upload.file);
        cleanup_staging_file(&upload.staging_path);
        return Err(format!(
            "Could not securely finish the uploaded file: {error}"
        ));
    }
    let actual_digest = hex::encode(upload.digest.finalize());
    if actual_digest != upload.plan.sha256 {
        cleanup_staging_file(&upload.staging_path);
        return Err("The uploaded file failed its SHA-256 integrity check.".to_owned());
    }
    drop(upload.file);

    let final_path = publish_without_overwrite(
        &upload.staging_path,
        &upload.plan.destination_directory,
        &upload.plan.filename,
    )?;
    Ok((upload.plan, final_path))
}

fn publish_without_overwrite(
    staging_path: &Path,
    directory: &Path,
    filename: &str,
) -> Result<PathBuf, String> {
    let requested = Path::new(filename);
    let stem = requested
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("upload");
    let extension = requested
        .extension()
        .and_then(|extension| extension.to_str());
    for suffix in 0..10_000u32 {
        let candidate_name = if suffix == 0 {
            filename.to_owned()
        } else if let Some(extension) = extension {
            format!("{stem} ({suffix}).{extension}")
        } else {
            format!("{stem} ({suffix})")
        };
        let candidate = directory.join(candidate_name);
        match std::fs::hard_link(staging_path, &candidate) {
            Ok(()) => {
                cleanup_staging_file(staging_path);
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                cleanup_staging_file(staging_path);
                return Err(format!("Could not publish the uploaded file: {error}"));
            }
        }
    }
    cleanup_staging_file(staging_path);
    Err("Could not choose a collision-free filename for the upload.".to_owned())
}

fn cleanup_staging_file(path: &Path) {
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!("could not clean up Remote Control upload staging file: {error}");
        }
    }
}

fn workspace_fingerprint(snapshot: &clinch_companion_protocol::WorkspaceSnapshot) -> Vec<u8> {
    let mut snapshot = snapshot.clone();
    snapshot.sequence = 0;
    serde_json::to_vec(&snapshot).unwrap_or_default()
}

fn snapshot_workspace_fingerprint(envelope: &ServerEnvelope) -> Option<Vec<u8>> {
    match &envelope.payload {
        ServerMessage::Snapshot(snapshot) => Some(workspace_fingerprint(snapshot)),
        _ => None,
    }
}

fn protocol_command_accepted(request_id: Option<RequestId>, _detail: &str) -> ServerEnvelope {
    ServerEnvelope {
        version: PROTOCOL_VERSION,
        request_id,
        sequence: None,
        payload: ServerMessage::CommandAccepted {
            workspace_revision: 0,
        },
    }
}

async fn send_json(
    socket: &mut WebSocket,
    connection_sequence: &mut u64,
    value: &ServerEnvelope,
) -> Result<(), axum::Error> {
    *connection_sequence = connection_sequence.saturating_add(1);
    let mut value = value.clone();
    value.sequence = Some(*connection_sequence);
    let encoded = serde_json::to_string(&value).unwrap_or_else(|_| {
        "{\"version\":1,\"request_id\":null,\"sequence\":null,\"payload\":{\"type\":\"error\",\"data\":{\"code\":\"internal\",\"message\":\"Encoding failed.\",\"retryable\":false,\"current_revision\":null}}}".to_owned()
    });
    socket.send(Message::Text(encoded.into())).await
}

async fn static_index(State(state): State<GatewayState>, headers: HeaderMap) -> Response {
    static_response(&state, &headers, "index.html", true).await
}

async fn static_asset(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    AxumPath(asset): AxumPath<String>,
) -> Response {
    if asset.ends_with(".map") {
        return StatusCode::NOT_FOUND.into_response();
    }
    static_response(&state, &headers, &asset, false).await
}

async fn static_response(
    state: &GatewayState,
    headers: &HeaderMap,
    asset: &str,
    force_index: bool,
) -> Response {
    if !state.security.validate_host(headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if asset
        .split('/')
        .any(|part| part == ".." || part.contains(['\\', '\0']))
    {
        return StatusCode::NOT_FOUND.into_response();
    }

    let mut relative = if force_index { "index.html" } else { asset };
    let mut candidate = state.assets_root.join(relative);
    if !candidate.is_file()
        && headers
            .get(ACCEPT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|accept| accept.contains("text/html"))
    {
        relative = "index.html";
        candidate = state.assets_root.join(relative);
    }
    let canonical_root = match tokio::fs::canonicalize(&*state.assets_root).await {
        Ok(root) => root,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let canonical = match tokio::fs::canonicalize(&candidate).await {
        Ok(candidate) if candidate.starts_with(&canonical_root) => candidate,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let bytes = match tokio::fs::read(canonical).await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let mime = mime_guess::from_path(relative).first_or_octet_stream();
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_str(mime.as_ref())
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    let cache = if relative == "index.html"
        || relative.ends_with("sw.js")
        || relative.ends_with("manifest.webmanifest")
    {
        "no-cache"
    } else if relative.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=3600"
    };
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    add_security_headers(&mut response);
    response
}

fn validate_api_headers(state: &GatewayState, headers: &HeaderMap) -> bool {
    state.security.validate_host(headers) && state.security.validate_origin(headers)
}

fn session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get(COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|cookie| cookie.trim().split_once('='))
        .find_map(|(name, value)| (name == SESSION_COOKIE_NAME).then(|| value.to_owned()))
}

fn api_json<T: Serialize>(status: StatusCode, value: &T) -> Response {
    let mut response = (status, Json(value)).into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    add_security_headers(&mut response);
    response
}

#[derive(Serialize)]
struct HttpError<'a> {
    error: HttpErrorBody<'a>,
}

#[derive(Serialize)]
struct HttpErrorBody<'a> {
    code: &'a str,
    message: &'a str,
}

fn http_error(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    api_json(
        status,
        &HttpError {
            error: HttpErrorBody { code, message },
        },
    )
}

fn pairing_error(error: PairingError) -> Response {
    match error.kind() {
        PairingErrorKind::InvalidRequest => http_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "The request is invalid.",
        ),
        PairingErrorKind::Unauthorized => http_error(
            StatusCode::UNAUTHORIZED,
            "authentication",
            "The pairing or authentication proof is invalid.",
        ),
        PairingErrorKind::NotFound => http_error(
            StatusCode::NOT_FOUND,
            "not_found",
            "The pairing or authorization record was not found.",
        ),
        PairingErrorKind::Expired => http_error(
            StatusCode::GONE,
            "expired",
            "The pairing or authorization record expired.",
        ),
        PairingErrorKind::AlreadyUsed => http_error(
            StatusCode::CONFLICT,
            "already_used",
            "The pairing invitation or claim was already used.",
        ),
        PairingErrorKind::Capacity => http_error(
            StatusCode::TOO_MANY_REQUESTS,
            "capacity",
            "Remote Control has reached its connection limit.",
        ),
        PairingErrorKind::RateLimited => http_error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many attempts. Try again shortly.",
        ),
        PairingErrorKind::Revoked => http_error(
            StatusCode::FORBIDDEN,
            "revoked",
            "This phone's authorization was revoked.",
        ),
        PairingErrorKind::Internal => http_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "Remote Control is temporarily unavailable.",
        ),
    }
}

fn protocol_error(
    request_id: Option<clinch_companion_protocol::RequestId>,
    code: ProtocolErrorCode,
    message: &str,
) -> ServerEnvelope {
    ServerEnvelope {
        version: PROTOCOL_VERSION,
        request_id,
        sequence: None,
        payload: ServerMessage::Error(ProtocolError {
            code,
            message: message.to_owned(),
            retryable: false,
            current_revision: None,
        }),
    }
}

fn add_security_headers(response: &mut Response) {
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(STATIC_CSP),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(self), microphone=(), geolocation=(), payment=()"),
    );
}

#[cfg(test)]
#[path = "gateway_tests.rs"]
mod tests;

//! Main-thread command boundary between the companion gateway and Clinch's UI models.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher as _};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use base64::engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use chrono::{Duration, Utc};
use clinch_companion_protocol::{
    javascript_safe_integer, AcquireWriterLease, AgentProvider, AgentState, AppInstanceId,
    AuthSessionId, Capability, ClientEnvelope, ClientMessage, ConnectionPath, CreateProject,
    CreateSession, CreateTask, DeleteTask, DeviceId, HostSnapshot, InterruptTerminal, LaunchTask,
    PaneKind, PaneSnapshot, ProjectActivity, ProjectBadgeSnapshot, ProjectSnapshot, ProtocolError,
    ProtocolErrorCode, QuickInsertDescriptor, QuickInsertKind, QuickInsertPreviewRequest,
    QuickInsertSubmit, RawTerminalInput, RecentAgentSessionSnapshot, RequestId, ResumeSession,
    ServerEnvelope, ServerMessage, SessionKind, SetTerminalSizePin, SubmitComposerText, TabKind,
    TabSnapshot, TargetRef, TaskId, TaskSnapshot, TerminalDimensions, TerminalKey,
    TerminalKeyInput, TerminalResize, TerminalSnapshot, TerminalStreamId, UploadBegin, UploadId,
    UploadReady, UsageLimitWindowSnapshot, UsageSnapshot, UsageState, UsageTokenWindowSnapshot,
    WorkspaceChanged, WorkspaceSnapshot, WriterLeaseSnapshot, MAX_IDEMPOTENCY_RESULTS_PER_SESSION,
    MAX_JAVASCRIPT_SAFE_INTEGER, MAX_OPAQUE_ID_BYTES, MAX_PATH_BYTES, MAX_TERMINAL_SNAPSHOT_BYTES,
    MAX_UPLOAD_CHUNK_BYTES, PROTOCOL_VERSION, WRITER_LEASE_TTL_SECS,
};
use instant::Instant;
use rand::rngs::OsRng;
use rand::RngCore as _;
use warpui::{
    AppContext, Entity, EntityId, ModelContext, SingletonEntity, TypedActionView, ViewHandle,
};

use super::pairing::{PairingManager, SessionAuthorization};
use crate::agent_resume::AgentResumeProvider;
use crate::ai::blocklist::agent_view::toolbar_item::AgentToolbarItemKind;
use crate::ai::blocklist::usage::CliAgentUsageModel;
use crate::pane_group::{ActivationReason, PaneGroup, PaneGroupAction, PaneId};
use crate::project_window::{ProjectId, ProjectWindow};
use crate::root_view::RootView;
use crate::settings::CliAgentUsageSettings;
use crate::terminal::cli_agent_sessions::{CLIAgentSessionStatus, CLIAgentSessionsModel};
use crate::terminal::session_settings::{SessionSettings, ToolbarChipSelection as _};
use crate::terminal::view::TerminalViewState;
use crate::terminal::{CLIAgent, Event as TerminalViewEvent, TerminalView};
use crate::workspace::task::{WorkspaceTaskAgent, WorkspaceTaskId};
use crate::workspace::Workspace;
use crate::AgentNotificationsModel;

/// Extra connection-scoped work accompanying a protocol response.
pub struct AdapterReply {
    pub envelope: ServerEnvelope,
    pub terminal_stream: Option<TerminalOutputStream>,
    pub upload_plan: Option<UploadPlan>,
}

impl AdapterReply {
    fn envelope(envelope: ServerEnvelope) -> Self {
        Self {
            envelope,
            terminal_stream: None,
            upload_plan: None,
        }
    }
}

pub struct TerminalOutputStream {
    pub stream_id: TerminalStreamId,
    pub receiver: async_broadcast::Receiver<Arc<Vec<u8>>>,
}

#[derive(Clone)]
pub struct UploadPlan {
    pub upload_id: UploadId,
    pub target: TargetRef,
    pub destination_directory: PathBuf,
    pub filename: String,
    pub size: u64,
    pub sha256: String,
}

pub struct UploadCompletion {
    pub request_id: Option<RequestId>,
    pub upload_id: UploadId,
    pub target: TargetRef,
    pub expected_directory: PathBuf,
    pub final_path: String,
    pub authorization: SessionAuthorization,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TargetKey {
    project_id: String,
    tab_id: String,
    pane_id: String,
}

impl From<&TargetRef> for TargetKey {
    fn from(target: &TargetRef) -> Self {
        Self {
            project_id: target.project_id.clone(),
            tab_id: target.tab_id.clone(),
            pane_id: target.pane_id.clone(),
        }
    }
}

#[derive(Clone)]
struct WriterLease {
    session_id: AuthSessionId,
    device_id: clinch_companion_protocol::DeviceId,
    device_name: String,
    expires_at: chrono::DateTime<Utc>,
}

impl WriterLease {
    fn snapshot(&self) -> WriterLeaseSnapshot {
        WriterLeaseSnapshot {
            device_id: self.device_id,
            device_name: self.device_name.clone(),
            expires_at: self.expires_at,
        }
    }
}

/// A lease never expires while its holder's session is still connected: a phone that is reading
/// a long CLI response sends no writes, and dropping its lease would revert the PTY to desktop
/// dimensions mid-conversation. `expires_at` is therefore only honored once the holder is no
/// longer in the connected set (an unclean disconnect that skipped `session_disconnected`).
fn writer_lease_expired(
    connected_sessions: &HashSet<AuthSessionId>,
    lease: &WriterLease,
    now: chrono::DateTime<Utc>,
) -> bool {
    lease.expires_at <= now && !connected_sessions.contains(&lease.session_id)
}

/// A lease only blocks OTHER devices, and only while its holder is still connected. The same
/// device reconnecting under a new session (a page reload, a phone returning from background)
/// adopts its own lease instead of being locked out by a session it can no longer resume, and
/// a lease riding out the disconnect grace window never makes a second device wait for a
/// holder whose socket is already gone.
fn writer_lease_blocks(
    connected_sessions: &HashSet<AuthSessionId>,
    lease: &WriterLease,
    authorization: &SessionAuthorization,
) -> bool {
    lease.session_id != authorization.session_id
        && lease.device_id != authorization.device_id
        && connected_sessions.contains(&lease.session_id)
}

/// Whether a remote viewport may impose its own dimensions on a pane.
///
/// A PTY carries exactly one `winsize`, so a pane cannot be one width on the Mac and another on a
/// phone. Whoever is looking at it on the Mac owns that width; a remote device takes it only for a
/// pane the Mac is not showing, or after deliberately pinning it from that device.
fn remote_may_size_pane(
    desktop_watching: bool,
    pinned_by: Option<&DeviceId>,
    device_id: &DeviceId,
) -> bool {
    match pinned_by {
        Some(pinned_by) => pinned_by == device_id,
        None => !desktop_watching,
    }
}

#[derive(Clone)]
struct ResolvedTarget {
    project_window: ViewHandle<ProjectWindow>,
    project_id: ProjectId,
    workspace: ViewHandle<Workspace>,
    tab_index: usize,
    pane_group: ViewHandle<PaneGroup>,
    pane_id: PaneId,
    terminal: ViewHandle<TerminalView>,
}

#[derive(Clone)]
struct ResolvedProject {
    project_window: ViewHandle<ProjectWindow>,
    project_id: ProjectId,
    workspace: ViewHandle<Workspace>,
}

#[derive(Clone)]
struct ResolvedQuickInsert {
    descriptor: QuickInsertDescriptor,
    text: String,
}

pub struct WorkspaceAdapter {
    app_instance_id: AppInstanceId,
    revision: u64,
    sequence: u64,
    quick_insert_salt: u64,
    last_topology_fingerprint: Option<u64>,
    pairing: PairingManager,
    writer_leases: HashMap<TargetKey, WriterLease>,
    /// Panes a remote device deliberately claimed the PTY width for. Without an entry here a
    /// remote viewport only sizes panes the Mac is not currently showing, so simply looking at
    /// a session from a phone can never reshape the pane someone is working in.
    remote_size_pins: HashMap<TargetKey, DeviceId>,
    connected_sessions: HashSet<AuthSessionId>,
    terminal_subscriptions: HashSet<EntityId>,
    idempotency: HashMap<AuthSessionId, VecDeque<(RequestId, ServerEnvelope)>>,
    recent_agent_sessions: Vec<RecentAgentSessionSnapshot>,
    recent_agent_sessions_refreshed_at: Option<Instant>,
    recent_agent_sessions_refresh_in_flight: bool,
}

impl Entity for WorkspaceAdapter {
    type Event = ();
}

impl SingletonEntity for WorkspaceAdapter {}

impl WorkspaceAdapter {
    pub fn new(pairing: PairingManager, _ctx: &mut ModelContext<Self>) -> Self {
        let revision = initial_workspace_revision(OsRng.next_u64());
        Self {
            app_instance_id: AppInstanceId::new(),
            revision,
            sequence: 0,
            quick_insert_salt: OsRng.next_u64(),
            last_topology_fingerprint: None,
            pairing,
            writer_leases: HashMap::new(),
            remote_size_pins: HashMap::new(),
            connected_sessions: HashSet::new(),
            terminal_subscriptions: HashSet::new(),
            idempotency: HashMap::new(),
            recent_agent_sessions: Vec::new(),
            recent_agent_sessions_refreshed_at: None,
            recent_agent_sessions_refresh_in_flight: false,
        }
    }

    pub fn initial_snapshot(&mut self, ctx: &mut ModelContext<Self>) -> ServerEnvelope {
        let snapshot = self.snapshot(ctx);
        self.response(None, ServerMessage::Snapshot(snapshot))
    }

    /// Returns an unsequenced authoritative snapshot for connection-local change detection.
    pub fn poll_snapshot(&mut self, ctx: &mut ModelContext<Self>) -> WorkspaceSnapshot {
        self.snapshot(ctx)
    }

    pub fn workspace_changed(&mut self, snapshot: WorkspaceSnapshot) -> ServerEnvelope {
        self.response(
            None,
            ServerMessage::WorkspaceChanged(WorkspaceChanged { snapshot }),
        )
    }

    pub fn handle_envelope(
        &mut self,
        envelope: ClientEnvelope,
        authorization: SessionAuthorization,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        if let Err(error) = envelope.validate() {
            return AdapterReply::envelope(self.error(
                Some(envelope.request_id),
                ProtocolErrorCode::InvalidRequest,
                error.to_string(),
                false,
            ));
        }
        if let Some(cached) = self.cached(authorization.session_id, envelope.request_id) {
            return AdapterReply::envelope(cached);
        }
        if let Some(required) = required_capability(&envelope.payload) {
            if !authorization.capabilities.contains(&required) {
                return AdapterReply::envelope(self.error(
                    Some(envelope.request_id),
                    ProtocolErrorCode::CapabilityDenied,
                    "This phone is not authorized for that action.".to_owned(),
                    false,
                ));
            }
        }

        let request_id = envelope.request_id;
        let cache_response = is_idempotent_mutation(&envelope.payload);
        let reply = match envelope.payload {
            ClientMessage::Ping => {
                AdapterReply::envelope(self.response(Some(request_id), ServerMessage::Pong))
            }
            ClientMessage::RequestSnapshot => {
                let snapshot = self.snapshot(ctx);
                AdapterReply::envelope(
                    self.response(Some(request_id), ServerMessage::Snapshot(snapshot)),
                )
            }
            ClientMessage::SelectTarget(message) => {
                self.select_target(request_id, message.target, message.workspace_revision, ctx)
            }
            ClientMessage::AcquireWriterLease(message) => {
                self.acquire_writer_lease(request_id, message, authorization.clone(), ctx)
            }
            ClientMessage::ReleaseWriterLease(message) => {
                self.release_writer_lease(request_id, message.target, &authorization, ctx)
            }
            ClientMessage::SubmitComposerText(message) => {
                self.submit_composer(request_id, message, authorization.clone(), ctx)
            }
            ClientMessage::RawTerminalInput(message) => {
                self.raw_terminal_input(request_id, message, authorization.clone(), ctx)
            }
            ClientMessage::InterruptTerminal(message) => {
                self.interrupt_terminal(request_id, message, authorization.clone(), ctx)
            }
            ClientMessage::TerminalResize(message) => {
                self.resize_terminal(request_id, message, authorization.clone(), ctx)
            }
            ClientMessage::SetTerminalSizePin(message) => {
                self.set_terminal_size_pin(request_id, message, authorization.clone(), ctx)
            }
            ClientMessage::TerminalKey(message) => {
                self.terminal_key(request_id, message, authorization.clone(), ctx)
            }
            ClientMessage::CreateProject(message) => self.create_project(request_id, message, ctx),
            ClientMessage::CreateSession(message) => self.create_session(request_id, message, ctx),
            ClientMessage::ResumeSession(message) => self.resume_session(request_id, message, ctx),
            ClientMessage::CreateTask(message) => self.create_task(request_id, message, ctx),
            ClientMessage::DeleteTask(message) => self.delete_task(request_id, message, ctx),
            ClientMessage::LaunchTask(message) => self.launch_task(request_id, message, ctx),
            ClientMessage::QuickInsertPreview(message) => {
                self.quick_insert_preview(request_id, message, ctx)
            }
            ClientMessage::QuickInsertSubmit(message) => {
                self.quick_insert_submit(request_id, message, authorization.clone(), ctx)
            }
            ClientMessage::UploadBegin(message) => {
                self.begin_upload(request_id, message, authorization.clone(), ctx)
            }
            ClientMessage::UploadCommit(_) | ClientMessage::UploadCancel(_) => {
                AdapterReply::envelope(self.error(
                    Some(request_id),
                    ProtocolErrorCode::UploadRejected,
                    "No matching upload is active on this connection.".to_owned(),
                    false,
                ))
            }
            // The gateway answers unpairing before the adapter ever sees it; reaching here
            // means a bug in the gateway dispatch, so fail loudly rather than pretend.
            ClientMessage::UnpairDevice => AdapterReply::envelope(self.error(
                Some(request_id),
                ProtocolErrorCode::Internal,
                "Unpairing is handled by the connection, not the workspace.".to_owned(),
                false,
            )),
            ClientMessage::Disconnect => AdapterReply::envelope(self.response(
                Some(request_id),
                ServerMessage::CommandAccepted {
                    workspace_revision: self.revision,
                },
            )),
        };

        if cache_response {
            self.remember(authorization.session_id, request_id, reply.envelope.clone());
        }
        reply
    }

    pub fn complete_upload(
        &mut self,
        completion: UploadCompletion,
        ctx: &mut ModelContext<Self>,
    ) -> ServerEnvelope {
        let UploadCompletion {
            request_id,
            upload_id,
            target,
            expected_directory,
            final_path,
            authorization,
        } = completion;
        let resolved = match self.resolve_valid_target(&target, None, ctx) {
            Ok(resolved) => resolved,
            Err((code, message, retryable)) => {
                return self.error(request_id, code, message, retryable);
            }
        };
        if let Err(envelope) = self.ensure_writer(&target, &authorization, request_id, false) {
            return *envelope;
        }
        let current_directory = resolved
            .terminal
            .read(ctx, |terminal, ctx| terminal.pwd_if_local(ctx))
            .and_then(|cwd| canonical_local_directory(&cwd).ok());
        if current_directory.as_ref() != Some(&expected_directory) {
            return self.error(
                request_id,
                ProtocolErrorCode::UploadRejected,
                "The terminal directory changed before the upload completed.".to_owned(),
                true,
            );
        }
        let inserted_path = resolved.terminal.update(ctx, |terminal, ctx| {
            let inserted_path = terminal.remote_control_escape_inserted_path(&final_path, ctx);
            terminal.remote_control_insert_text(inserted_path.clone(), ctx);
            inserted_path
        });
        self.response(
            request_id,
            ServerMessage::UploadCompleted(clinch_companion_protocol::UploadCompleted {
                upload_id,
                inserted_path,
            }),
        )
    }

    /// Marks a companion session's WebSocket as live so its writer leases stay valid while it
    /// is merely reading. Renewing only on writes made an idle phone lose the lease after
    /// `WRITER_LEASE_TTL_SECS`, which silently reverted the PTY to desktop dimensions
    /// mid-conversation and garbled every subsequent CLI repaint on the phone.
    ///
    /// A returning device usually arrives under a fresh auth session while its leases still
    /// reference the old one, so rebind them here; otherwise an adopted lease would silently
    /// lapse on the TTL backstop mid-use even though its holder is connected.
    pub fn session_connected(
        &mut self,
        session_id: AuthSessionId,
        device_id: clinch_companion_protocol::DeviceId,
    ) {
        self.connected_sessions.insert(session_id);
        let now = Utc::now();
        for lease in self.writer_leases.values_mut() {
            if lease.device_id == device_id && !self.connected_sessions.contains(&lease.session_id)
            {
                lease.session_id = session_id;
                lease.expires_at = now + Duration::seconds(WRITER_LEASE_TTL_SECS as i64);
            }
        }
    }

    /// Phones close their socket for something as small as a notification peek, so a
    /// disconnect must not restore desktop PTY sizing immediately: every restore-and-resize
    /// round trip makes full-screen CLIs repaint while dimensions are in flux, which bakes
    /// word-splits into the transcript. Instead the disconnect re-arms the lease TTL as a
    /// grace window; `prune_writer_leases` restores desktop dimensions only if the device
    /// stays away, and the gateway schedules a delayed sweep so that happens even when no
    /// other client is left polling.
    pub fn session_disconnected(
        &mut self,
        session_id: AuthSessionId,
        _ctx: &mut ModelContext<Self>,
    ) {
        self.connected_sessions.remove(&session_id);
        let now = Utc::now();
        for lease in self.writer_leases.values_mut() {
            if lease.session_id == session_id {
                lease.expires_at = now + Duration::seconds(WRITER_LEASE_TTL_SECS as i64);
            }
        }
        self.idempotency.remove(&session_id);
    }

    /// Restores desktop sizing for leases whose grace window lapsed. Connected clients drive
    /// pruning through their snapshot polls, but after the last client disconnects nothing
    /// polls, so the gateway schedules one of these per disconnect.
    pub fn sweep_writer_leases(&mut self, ctx: &mut ModelContext<Self>) {
        self.prune_writer_leases(ctx);
    }

    pub fn all_sessions_disconnected(&mut self, ctx: &mut ModelContext<Self>) {
        self.connected_sessions.clear();
        let targets = self.writer_leases.keys().cloned().collect::<Vec<_>>();
        self.writer_leases.clear();
        self.remote_size_pins.clear();
        self.idempotency.clear();
        for target in targets {
            if let Some(resolved) = self.resolve_target_key(&target, ctx) {
                resolved.terminal.update(ctx, |terminal, ctx| {
                    terminal.remote_control_restore_desktop_size(ctx);
                });
            }
        }
    }

    fn select_target(
        &mut self,
        request_id: RequestId,
        target: TargetRef,
        _expected_revision: u64,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let resolved = match self.resolve_valid_target(&target, None, ctx) {
            Ok(resolved) => resolved,
            Err((code, message, retryable)) => {
                return AdapterReply::envelope(self.error(
                    Some(request_id),
                    code,
                    message,
                    retryable,
                ));
            }
        };
        if !resolved
            .terminal
            .read(ctx, |terminal, _| terminal.remote_control_is_ready())
        {
            return AdapterReply::envelope(self.error(
                Some(request_id),
                ProtocolErrorCode::ResyncRequired,
                "This terminal is still starting. Clinch will connect it automatically when it is ready."
                    .to_owned(),
                true,
            ));
        }

        let stream_id = TerminalStreamId::new();
        let (data, dimensions, receiver, zero_width_prompt) =
            resolved.terminal.read(ctx, |terminal, _| {
                // Subscribe before activating the desktop target. Activating an inactive terminal can
                // make its shell or full-screen CLI repaint immediately; subscribing afterwards made
                // that first redraw timing-dependent and occasionally left a restored phone tab blank.
                // The snapshot is captured before activation, so every subsequent activation byte is
                // represented exactly once by the stream rather than duplicated in both handoffs.
                let receiver = terminal.remote_control_pty_reads();
                let data = terminal.remote_control_scrollback_bytes(MAX_TERMINAL_SNAPSHOT_BYTES);
                let (columns, rows) = terminal.remote_control_dimensions();
                let zero_width_prompt = terminal.remote_control_zero_width_prompt();
                (
                    data,
                    TerminalDimensions { columns, rows },
                    receiver,
                    zero_width_prompt,
                )
            });
        resolved.project_window.update(ctx, |project_window, ctx| {
            project_window.activate_project(resolved.project_id, ctx);
        });
        resolved.workspace.update(ctx, |workspace, ctx| {
            workspace.activate_tab(resolved.tab_index, ctx);
        });
        resolved.pane_group.update(ctx, |pane_group, ctx| {
            pane_group.handle_action(
                &PaneGroupAction::Activate(resolved.pane_id, ActivationReason::Click),
                ctx,
            );
        });
        // Selecting the already-active desktop target does not otherwise produce any PTY bytes.
        // Ask the PTY to repaint at its existing size so a sparse block snapshot (for example an
        // idle shell prompt) is still visible after a browser refresh. The receiver above is
        // already active, so this redraw is part of the new stream with no handoff gap.
        resolved.terminal.update(ctx, |terminal, ctx| {
            terminal.remote_control_refresh_size(ctx);
        });

        let snapshot = TerminalSnapshot {
            target: target.clone(),
            stream_id,
            workspace_revision: self.revision,
            terminal_sequence: 0,
            data_base64: BASE64_STANDARD.encode(data),
            dimensions,
            zero_width_prompt,
        };
        AdapterReply {
            envelope: self.response(Some(request_id), ServerMessage::TerminalSnapshot(snapshot)),
            terminal_stream: Some(TerminalOutputStream {
                stream_id,
                receiver,
            }),
            upload_plan: None,
        }
    }

    fn acquire_writer_lease(
        &mut self,
        request_id: RequestId,
        message: AcquireWriterLease,
        authorization: SessionAuthorization,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        if let Err((code, message_text, retryable)) =
            self.resolve_valid_target(&message.target, None, ctx)
        {
            return AdapterReply::envelope(self.error(
                Some(request_id),
                code,
                message_text,
                retryable,
            ));
        }
        match self.ensure_writer(
            &message.target,
            &authorization,
            Some(request_id),
            message.takeover,
        ) {
            Ok(lease) => AdapterReply::envelope(self.response(
                Some(request_id),
                ServerMessage::WriterLeaseChanged {
                    target: message.target,
                    lease: Some(lease),
                },
            )),
            Err(error) => AdapterReply::envelope(*error),
        }
    }

    fn release_writer_lease(
        &mut self,
        request_id: RequestId,
        target: TargetRef,
        authorization: &SessionAuthorization,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let key = TargetKey::from(&target);
        let owned = self
            .writer_leases
            .get(&key)
            .is_some_and(|lease| lease.session_id == authorization.session_id);
        if owned {
            self.release_pane_control(&key);
            if let Some(resolved) = self.resolve_target_key(&key, ctx) {
                resolved.terminal.update(ctx, |terminal, ctx| {
                    terminal.remote_control_restore_desktop_size(ctx);
                });
            }
        }
        AdapterReply::envelope(self.response(
            Some(request_id),
            ServerMessage::WriterLeaseChanged {
                target,
                lease: self.writer_leases.get(&key).map(WriterLease::snapshot),
            },
        ))
    }

    fn submit_composer(
        &mut self,
        request_id: RequestId,
        message: SubmitComposerText,
        authorization: SessionAuthorization,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let resolved =
            match self.resolve_valid_target(&message.target, Some(message.workspace_revision), ctx)
            {
                Ok(resolved) => resolved,
                Err(error) => return AdapterReply::envelope(self.target_error(request_id, error)),
            };
        if let Err(error) =
            self.ensure_writer(&message.target, &authorization, Some(request_id), false)
        {
            return AdapterReply::envelope(*error);
        }
        let submitted = resolved.terminal.update(ctx, |terminal, ctx| {
            terminal.remote_control_submit_text(message.text, ctx)
        });
        if !submitted {
            return AdapterReply::envelope(self.error(
                Some(request_id),
                ProtocolErrorCode::InvalidRequest,
                "That agent is not currently accepting prompt input.".to_owned(),
                true,
            ));
        }
        AdapterReply::envelope(self.command_accepted(request_id))
    }

    fn raw_terminal_input(
        &mut self,
        request_id: RequestId,
        message: RawTerminalInput,
        authorization: SessionAuthorization,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let resolved =
            match self.resolve_valid_target(&message.target, Some(message.workspace_revision), ctx)
            {
                Ok(resolved) => resolved,
                Err(error) => return AdapterReply::envelope(self.target_error(request_id, error)),
            };
        if let Err(error) =
            self.ensure_writer(&message.target, &authorization, Some(request_id), false)
        {
            return AdapterReply::envelope(*error);
        }
        let Ok(bytes) = BASE64_STANDARD.decode(message.data_base64) else {
            return AdapterReply::envelope(self.error(
                Some(request_id),
                ProtocolErrorCode::InvalidRequest,
                "Terminal input is not valid base64.".to_owned(),
                false,
            ));
        };
        resolved.terminal.update(ctx, |terminal, ctx| {
            terminal.remote_control_write_bytes(bytes, ctx);
        });
        AdapterReply::envelope(self.command_accepted(request_id))
    }

    fn interrupt_terminal(
        &mut self,
        request_id: RequestId,
        message: InterruptTerminal,
        authorization: SessionAuthorization,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        self.write_fixed_terminal_bytes(
            request_id,
            message.target,
            message.workspace_revision,
            vec![0x03],
            authorization,
            ctx,
        )
    }

    fn terminal_key(
        &mut self,
        request_id: RequestId,
        message: TerminalKeyInput,
        authorization: SessionAuthorization,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let bytes = match message.key {
            TerminalKey::Escape => b"\x1b".to_vec(),
            TerminalKey::Tab => b"\t".to_vec(),
            TerminalKey::ArrowUp => b"\x1b[A".to_vec(),
            TerminalKey::ArrowDown => b"\x1b[B".to_vec(),
            TerminalKey::ArrowLeft => b"\x1b[D".to_vec(),
            TerminalKey::ArrowRight => b"\x1b[C".to_vec(),
            TerminalKey::ControlD => vec![0x04],
            TerminalKey::Home => b"\x1b[H".to_vec(),
            TerminalKey::End => b"\x1b[F".to_vec(),
            TerminalKey::PageUp => b"\x1b[5~".to_vec(),
            TerminalKey::PageDown => b"\x1b[6~".to_vec(),
            TerminalKey::Delete => b"\x1b[3~".to_vec(),
        };
        self.write_fixed_terminal_bytes(
            request_id,
            message.target,
            message.workspace_revision,
            bytes,
            authorization,
            ctx,
        )
    }

    fn write_fixed_terminal_bytes(
        &mut self,
        request_id: RequestId,
        target: TargetRef,
        expected_revision: u64,
        bytes: Vec<u8>,
        authorization: SessionAuthorization,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let resolved = match self.resolve_valid_target(&target, Some(expected_revision), ctx) {
            Ok(resolved) => resolved,
            Err(error) => return AdapterReply::envelope(self.target_error(request_id, error)),
        };
        if let Err(error) = self.ensure_writer(&target, &authorization, Some(request_id), false) {
            return AdapterReply::envelope(*error);
        }
        resolved.terminal.update(ctx, |terminal, ctx| {
            terminal.remote_control_write_bytes(bytes, ctx);
        });
        AdapterReply::envelope(self.command_accepted(request_id))
    }

    fn resize_terminal(
        &mut self,
        request_id: RequestId,
        message: TerminalResize,
        authorization: SessionAuthorization,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let resolved = match self.resolve_valid_target(&message.target, None, ctx) {
            Ok(resolved) => resolved,
            Err(error) => return AdapterReply::envelope(self.target_error(request_id, error)),
        };
        if let Err(error) =
            self.ensure_writer(&message.target, &authorization, Some(request_id), false)
        {
            return AdapterReply::envelope(*error);
        }
        let key = TargetKey::from(&message.target);
        let desktop_watching = self.desktop_watched_targets(ctx).contains(&key);
        if !remote_may_size_pane(
            desktop_watching,
            self.remote_size_pins.get(&key),
            &authorization.device_id,
        ) {
            // Someone is looking at this pane on the Mac, and a PTY has only one width. Accept
            // the request without reshaping the pane: the phone reads the authoritative
            // dimensions off the next pane snapshot and mirrors them instead of imposing its
            // own. Pinning from the phone is what makes taking that width deliberate.
            return AdapterReply::envelope(self.command_accepted(request_id));
        }
        resolved.terminal.update(ctx, |terminal, ctx| {
            terminal.remote_control_resize(
                message.dimensions.rows as usize,
                message.dimensions.columns as usize,
                ctx,
            );
        });
        AdapterReply::envelope(self.command_accepted(request_id))
    }

    /// Takes or gives back deliberate ownership of a pane's PTY width. Pinning is the only way a
    /// remote device reshapes a pane the Mac is showing, and unpinning immediately hands the
    /// width back if the Mac is watching.
    fn set_terminal_size_pin(
        &mut self,
        request_id: RequestId,
        message: SetTerminalSizePin,
        authorization: SessionAuthorization,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let resolved = match self.resolve_valid_target(&message.target, None, ctx) {
            Ok(resolved) => resolved,
            Err(error) => return AdapterReply::envelope(self.target_error(request_id, error)),
        };
        if let Err(error) =
            self.ensure_writer(&message.target, &authorization, Some(request_id), false)
        {
            return AdapterReply::envelope(*error);
        }
        let key = TargetKey::from(&message.target);
        if message.pinned {
            self.remote_size_pins.insert(key, authorization.device_id);
        } else {
            self.remote_size_pins.remove(&key);
            if self.desktop_watched_targets(ctx).contains(&key) {
                resolved.terminal.update(ctx, |terminal, ctx| {
                    terminal.remote_control_restore_desktop_size(ctx);
                });
            }
        }
        AdapterReply::envelope(self.command_accepted(request_id))
    }

    fn create_session(
        &mut self,
        request_id: RequestId,
        message: CreateSession,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let project = match self.resolve_valid_project(
            message.app_instance_id,
            &message.project_id,
            None,
            ctx,
        ) {
            Ok(project) => project,
            Err(error) => return AdapterReply::envelope(self.target_error(request_id, error)),
        };
        let cwd = match optional_local_directory(message.cwd) {
            Ok(cwd) => cwd,
            Err(message) => {
                return AdapterReply::envelope(self.error(
                    Some(request_id),
                    ProtocolErrorCode::InvalidRequest,
                    message,
                    false,
                ));
            }
        };

        project.project_window.update(ctx, |project_window, ctx| {
            project_window.activate_project(project.project_id, ctx);
        });
        match message.kind {
            SessionKind::Terminal => project.workspace.update(ctx, |workspace, ctx| {
                workspace.remote_control_open_terminal(cwd, ctx);
            }),
            SessionKind::ClaudeCode | SessionKind::Codex => {
                #[cfg(feature = "local_tty")]
                project.workspace.update(ctx, |workspace, ctx| {
                    let _ = workspace.remote_control_open_cli_agent(
                        if message.kind == SessionKind::ClaudeCode {
                            AgentResumeProvider::Claude
                        } else {
                            AgentResumeProvider::Codex
                        },
                        cwd,
                        message.initial_prompt,
                        ctx,
                    );
                });
                #[cfg(not(feature = "local_tty"))]
                {
                    return AdapterReply::envelope(self.error(
                        Some(request_id),
                        ProtocolErrorCode::CapabilityDenied,
                        "This build cannot create local CLI-agent sessions.".to_owned(),
                        false,
                    ));
                }
            }
        }
        self.bump_topology_revision();
        AdapterReply::envelope(self.command_accepted(request_id))
    }

    fn create_project(
        &mut self,
        request_id: RequestId,
        message: CreateProject,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let anchor = match self.resolve_valid_project(
            message.app_instance_id,
            &message.project_id,
            None,
            ctx,
        ) {
            Ok(project) => project,
            Err(error) => return AdapterReply::envelope(self.target_error(request_id, error)),
        };
        let cwd = match optional_local_directory(message.cwd) {
            Ok(cwd) => cwd,
            Err(message) => {
                return AdapterReply::envelope(self.error(
                    Some(request_id),
                    ProtocolErrorCode::InvalidRequest,
                    message,
                    false,
                ));
            }
        };
        let workspace = anchor.project_window.update(ctx, |project_window, ctx| {
            let project_id = project_window.add_project(ctx);
            project_window
                .projects()
                .find(|(id, _)| *id == project_id)
                .map(|(_, workspace)| workspace.clone())
        });
        let Some(workspace) = workspace else {
            return AdapterReply::envelope(self.error(
                Some(request_id),
                ProtocolErrorCode::Internal,
                "Clinch created the project but could not open its first terminal.".to_owned(),
                true,
            ));
        };
        workspace.update(ctx, |workspace, ctx| {
            workspace.remote_control_open_terminal(cwd, ctx);
        });
        self.bump_topology_revision();
        AdapterReply::envelope(self.command_accepted(request_id))
    }

    fn resume_session(
        &mut self,
        request_id: RequestId,
        message: ResumeSession,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let project = match self.resolve_valid_project(
            message.app_instance_id,
            &message.project_id,
            None,
            ctx,
        ) {
            Ok(project) => project,
            Err(error) => return AdapterReply::envelope(self.target_error(request_id, error)),
        };
        let cwd = match canonical_local_directory(&message.cwd) {
            Ok(cwd) => cwd,
            Err(message) => {
                return AdapterReply::envelope(self.error(
                    Some(request_id),
                    ProtocolErrorCode::InvalidRequest,
                    message,
                    false,
                ));
            }
        };
        project.project_window.update(ctx, |project_window, ctx| {
            project_window.activate_project(project.project_id, ctx);
        });
        #[cfg(feature = "local_tty")]
        let opened = project.workspace.update(ctx, |workspace, ctx| {
            workspace.remote_control_resume_cli_agent(
                match message.provider {
                    AgentProvider::ClaudeCode => AgentResumeProvider::Claude,
                    AgentProvider::Codex => AgentResumeProvider::Codex,
                },
                &message.durable_session_id,
                cwd,
                ctx,
            )
        });
        #[cfg(not(feature = "local_tty"))]
        let opened = false;
        if !opened {
            return AdapterReply::envelope(self.error(
                Some(request_id),
                ProtocolErrorCode::InvalidRequest,
                "The durable agent session ID is invalid for this provider.".to_owned(),
                false,
            ));
        }
        self.bump_topology_revision();
        AdapterReply::envelope(self.command_accepted(request_id))
    }

    fn create_task(
        &mut self,
        request_id: RequestId,
        message: CreateTask,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let project = match self.resolve_valid_project(
            message.app_instance_id,
            &message.project_id,
            Some(message.workspace_revision),
            ctx,
        ) {
            Ok(project) => project,
            Err(error) => return AdapterReply::envelope(self.target_error(request_id, error)),
        };
        let added = project.workspace.update(ctx, |workspace, ctx| {
            workspace.add_workspace_task(message.text, ctx)
        });
        if added.is_none() {
            return AdapterReply::envelope(self.error(
                Some(request_id),
                ProtocolErrorCode::InvalidRequest,
                "Task text must contain at least one non-whitespace character.".to_owned(),
                false,
            ));
        }
        self.bump_topology_revision();
        AdapterReply::envelope(self.command_accepted(request_id))
    }

    fn delete_task(
        &mut self,
        request_id: RequestId,
        message: DeleteTask,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let project = match self.resolve_valid_project(
            message.app_instance_id,
            &message.project_id,
            Some(message.workspace_revision),
            ctx,
        ) {
            Ok(project) => project,
            Err(error) => return AdapterReply::envelope(self.target_error(request_id, error)),
        };
        let removed = project.workspace.update(ctx, |workspace, ctx| {
            workspace.remove_workspace_task(WorkspaceTaskId(message.task_id.0), ctx)
        });
        if !removed {
            return AdapterReply::envelope(self.error(
                Some(request_id),
                ProtocolErrorCode::TargetGone,
                "That task no longer exists.".to_owned(),
                true,
            ));
        }
        self.bump_topology_revision();
        AdapterReply::envelope(self.command_accepted(request_id))
    }

    fn launch_task(
        &mut self,
        request_id: RequestId,
        message: LaunchTask,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let project = match self.resolve_valid_project(
            message.app_instance_id,
            &message.project_id,
            Some(message.workspace_revision),
            ctx,
        ) {
            Ok(project) => project,
            Err(error) => return AdapterReply::envelope(self.target_error(request_id, error)),
        };
        let task_id = WorkspaceTaskId(message.task_id.0);
        if !project
            .workspace
            .as_ref(ctx)
            .workspace_tasks()
            .iter()
            .any(|task| task.id == task_id)
        {
            return AdapterReply::envelope(self.error(
                Some(request_id),
                ProtocolErrorCode::TargetGone,
                "That task no longer exists.".to_owned(),
                true,
            ));
        }

        project.project_window.update(ctx, |project_window, ctx| {
            project_window.activate_project(project.project_id, ctx);
        });
        #[cfg(feature = "local_tty")]
        let launched = project.workspace.update(ctx, |workspace, ctx| {
            workspace.launch_workspace_task(
                task_id,
                match message.provider {
                    AgentProvider::ClaudeCode => WorkspaceTaskAgent::Claude,
                    AgentProvider::Codex => WorkspaceTaskAgent::Codex,
                },
                ctx,
            )
        });
        #[cfg(not(feature = "local_tty"))]
        let launched = false;
        if !launched {
            return AdapterReply::envelope(self.error(
                Some(request_id),
                ProtocolErrorCode::CapabilityDenied,
                "This build could not start a local CLI-agent session for that task.".to_owned(),
                false,
            ));
        }
        self.bump_topology_revision();
        AdapterReply::envelope(self.command_accepted(request_id))
    }

    fn quick_insert_preview(
        &mut self,
        request_id: RequestId,
        message: QuickInsertPreviewRequest,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        // Preview only returns text to the client for an xterm paste; it does not write to the PTY.
        // Re-resolve the exact target but tolerate a harmless topology revision change (for example,
        // immediately after opening a new tab). Submission remains revision-gated below.
        let resolved = match self.resolve_valid_target(&message.target, None, ctx) {
            Ok(resolved) => resolved,
            Err(error) => return AdapterReply::envelope(self.target_error(request_id, error)),
        };
        let Some(item) = self.resolve_quick_insert(
            &resolved.terminal,
            &message.item_id,
            message.configuration_revision,
            ctx,
        ) else {
            return AdapterReply::envelope(self.error(
                Some(request_id),
                ProtocolErrorCode::StaleQuickInsert,
                "Quick inserts changed on the Mac. Refresh and try again.".to_owned(),
                true,
            ));
        };
        AdapterReply::envelope(self.response(
            Some(request_id),
            ServerMessage::QuickInsertPreview(clinch_companion_protocol::QuickInsertPreview {
                item_id: item.descriptor.id,
                configuration_revision: item.descriptor.configuration_revision,
                text: item.text,
            }),
        ))
    }

    fn quick_insert_submit(
        &mut self,
        request_id: RequestId,
        message: QuickInsertSubmit,
        authorization: SessionAuthorization,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let resolved =
            match self.resolve_valid_target(&message.target, Some(message.workspace_revision), ctx)
            {
                Ok(resolved) => resolved,
                Err(error) => return AdapterReply::envelope(self.target_error(request_id, error)),
            };
        if let Err(error) =
            self.ensure_writer(&message.target, &authorization, Some(request_id), false)
        {
            return AdapterReply::envelope(*error);
        }
        let Some(item) = self.resolve_quick_insert(
            &resolved.terminal,
            &message.item_id,
            message.configuration_revision,
            ctx,
        ) else {
            return AdapterReply::envelope(self.error(
                Some(request_id),
                ProtocolErrorCode::StaleQuickInsert,
                "Quick inserts changed on the Mac. Refresh and try again.".to_owned(),
                true,
            ));
        };
        let submitted = resolved.terminal.update(ctx, |terminal, ctx| {
            terminal.remote_control_submit_text(item.text, ctx)
        });
        if !submitted {
            return AdapterReply::envelope(self.error(
                Some(request_id),
                ProtocolErrorCode::InvalidRequest,
                "That terminal is not accepting this quick insert.".to_owned(),
                true,
            ));
        }
        AdapterReply::envelope(self.command_accepted(request_id))
    }

    fn begin_upload(
        &mut self,
        request_id: RequestId,
        message: UploadBegin,
        authorization: SessionAuthorization,
        ctx: &mut ModelContext<Self>,
    ) -> AdapterReply {
        let resolved =
            match self.resolve_valid_target(&message.target, Some(message.workspace_revision), ctx)
            {
                Ok(resolved) => resolved,
                Err(error) => return AdapterReply::envelope(self.target_error(request_id, error)),
            };
        if let Err(error) =
            self.ensure_writer(&message.target, &authorization, Some(request_id), false)
        {
            return AdapterReply::envelope(*error);
        }
        let Some(cwd) = resolved
            .terminal
            .read(ctx, |terminal, ctx| terminal.pwd_if_local(ctx))
            .and_then(|cwd| canonical_local_directory(&cwd).ok())
        else {
            return AdapterReply::envelope(self.error(
                Some(request_id),
                ProtocolErrorCode::UploadRejected,
                "Uploads currently require a local terminal directory; SSH uploads are not yet supported."
                    .to_owned(),
                false,
            ));
        };
        let upload_id = UploadId::new();
        AdapterReply {
            envelope: self.response(
                Some(request_id),
                ServerMessage::UploadReady(UploadReady {
                    upload_id,
                    chunk_size: MAX_UPLOAD_CHUNK_BYTES as u32,
                }),
            ),
            terminal_stream: None,
            upload_plan: Some(UploadPlan {
                upload_id,
                target: message.target,
                destination_directory: cwd,
                filename: message.filename,
                size: message.size,
                sha256: message.sha256.to_ascii_lowercase(),
            }),
        }
    }

    fn resolve_valid_target(
        &mut self,
        target: &TargetRef,
        expected_revision: Option<u64>,
        ctx: &mut ModelContext<Self>,
    ) -> Result<ResolvedTarget, (ProtocolErrorCode, String, bool)> {
        if target.app_instance_id != self.app_instance_id {
            return Err((
                ProtocolErrorCode::TargetGone,
                "Clinch restarted; refresh and select the target again.".to_owned(),
                true,
            ));
        }
        if expected_revision.is_some_and(|revision| revision != self.revision) {
            return Err((
                ProtocolErrorCode::RevisionConflict,
                "The workspace changed; refresh before sending input.".to_owned(),
                true,
            ));
        }
        let resolved = self.resolve_target(target, ctx).ok_or((
            ProtocolErrorCode::TargetGone,
            "The requested project, tab, or terminal is no longer available.".to_owned(),
            true,
        ))?;
        self.subscribe_to_desktop_input(&resolved.terminal, ctx);
        Ok(resolved)
    }

    fn subscribe_to_desktop_input(
        &mut self,
        terminal: &ViewHandle<TerminalView>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self.terminal_subscriptions.insert(terminal.id()) {
            return;
        }
        ctx.subscribe_to_view(terminal, |adapter, terminal, event, ctx| {
            if matches!(event, TerminalViewEvent::DesktopInput) {
                adapter.preempt_writer_for_desktop_input(&terminal, ctx);
            }
        });
    }

    fn preempt_writer_for_desktop_input(
        &mut self,
        terminal: &ViewHandle<TerminalView>,
        ctx: &mut ModelContext<Self>,
    ) {
        let terminal_id = terminal.id();
        let targets = self
            .writer_leases
            .keys()
            .filter(|target| {
                self.resolve_target_key(target, ctx)
                    .is_none_or(|resolved| resolved.terminal.id() == terminal_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        if targets.is_empty() {
            return;
        }
        for target in targets {
            self.release_pane_control(&target);
        }
        terminal.update(ctx, |terminal, ctx| {
            terminal.remote_control_restore_desktop_size(ctx);
        });
        ctx.notify();
    }

    /// Drops a pane's writer lease together with any remote size pin. The two are deliberately
    /// released as one: a device that no longer holds control has no claim on the pane's width
    /// either, so a phone that wandered off can never leave the Mac stuck at phone dimensions.
    fn release_pane_control(&mut self, target: &TargetKey) {
        self.writer_leases.remove(target);
        self.remote_size_pins.remove(target);
    }

    fn resolve_target(
        &self,
        target: &TargetRef,
        ctx: &ModelContext<Self>,
    ) -> Option<ResolvedTarget> {
        for window_id in ctx.window_ids() {
            let Some(root) = ctx.root_view::<RootView>(window_id) else {
                continue;
            };
            let Some(project_window) = root.as_ref(ctx).project_window() else {
                continue;
            };
            let projects = project_window.read(ctx, |project_window, _| {
                project_window
                    .projects()
                    .map(|(id, workspace)| (id, workspace.clone()))
                    .collect::<Vec<_>>()
            });
            for (project_id, workspace) in projects {
                if project_id.opaque_id() != target.project_id {
                    continue;
                }
                let tabs = workspace.read(ctx, |workspace, _| {
                    workspace
                        .tab_views()
                        .enumerate()
                        .map(|(index, pane_group)| (index, pane_group.clone()))
                        .collect::<Vec<_>>()
                });
                for (tab_index, pane_group) in tabs {
                    if pane_group.id().to_string() != target.tab_id {
                        continue;
                    }
                    let pane = pane_group.read(ctx, |pane_group, ctx| {
                        pane_group
                            .visible_pane_ids()
                            .into_iter()
                            .find_map(|pane_id| {
                                if pane_opaque_id(pane_id) != target.pane_id {
                                    return None;
                                }
                                pane_group
                                    .terminal_view_from_pane_id(pane_id, ctx)
                                    .map(|terminal| (pane_id, terminal))
                            })
                    });
                    if let Some((pane_id, terminal)) = pane {
                        return Some(ResolvedTarget {
                            project_window: project_window.clone(),
                            project_id,
                            workspace,
                            tab_index,
                            pane_group,
                            pane_id,
                            terminal,
                        });
                    }
                }
            }
        }
        None
    }

    fn resolve_target_key(
        &self,
        key: &TargetKey,
        ctx: &ModelContext<Self>,
    ) -> Option<ResolvedTarget> {
        self.resolve_target(
            &TargetRef {
                app_instance_id: self.app_instance_id,
                project_id: key.project_id.clone(),
                tab_id: key.tab_id.clone(),
                pane_id: key.pane_id.clone(),
            },
            ctx,
        )
    }

    fn resolve_project(
        &self,
        project_id: &str,
        ctx: &ModelContext<Self>,
    ) -> Option<ResolvedProject> {
        for window_id in ctx.window_ids() {
            let Some(root) = ctx.root_view::<RootView>(window_id) else {
                continue;
            };
            let Some(project_window) = root.as_ref(ctx).project_window() else {
                continue;
            };
            let project_window_handle = project_window.clone();
            let found = project_window.read(ctx, |project_window, _| {
                project_window
                    .projects()
                    .find(|(id, _)| id.opaque_id() == project_id)
                    .map(|(id, workspace)| ResolvedProject {
                        project_window: project_window_handle.clone(),
                        project_id: id,
                        workspace: workspace.clone(),
                    })
            });
            if found.is_some() {
                return found;
            }
        }
        None
    }

    fn resolve_valid_project(
        &self,
        app_instance_id: AppInstanceId,
        project_id: &str,
        expected_revision: Option<u64>,
        ctx: &ModelContext<Self>,
    ) -> Result<ResolvedProject, (ProtocolErrorCode, String, bool)> {
        if app_instance_id != self.app_instance_id {
            return Err((
                ProtocolErrorCode::TargetGone,
                "Clinch restarted; refresh before creating a session.".to_owned(),
                true,
            ));
        }
        if expected_revision.is_some_and(|revision| revision != self.revision) {
            return Err((
                ProtocolErrorCode::RevisionConflict,
                "Projects changed on the Mac; refresh before creating a session.".to_owned(),
                true,
            ));
        }
        self.resolve_project(project_id, ctx).ok_or_else(|| {
            (
                ProtocolErrorCode::TargetGone,
                "That project is no longer open.".to_owned(),
                true,
            )
        })
    }

    /// `allow_takeover` is only true for an explicit `acquire_writer_lease` request that the
    /// client marked as deliberate user input; every implicit write path keeps it false so a
    /// client that wrongly believes it still owns the lease cannot silently displace the
    /// device that actually holds it.
    fn ensure_writer(
        &mut self,
        target: &TargetRef,
        authorization: &SessionAuthorization,
        request_id: Option<RequestId>,
        allow_takeover: bool,
    ) -> Result<WriterLeaseSnapshot, Box<ServerEnvelope>> {
        let key = TargetKey::from(target);
        if self
            .writer_leases
            .get(&key)
            .is_some_and(|lease| writer_lease_expired(&self.connected_sessions, lease, Utc::now()))
        {
            self.writer_leases.remove(&key);
        }
        if let Some(lease) = self.writer_leases.get(&key) {
            if !allow_takeover
                && writer_lease_blocks(&self.connected_sessions, lease, authorization)
            {
                let holder = lease.device_name.clone();
                return Err(Box::new(self.error(
                    request_id,
                    ProtocolErrorCode::WriterLeaseHeld,
                    format!("{holder} currently has control of this terminal."),
                    true,
                )));
            }
        }
        let lease = WriterLease {
            session_id: authorization.session_id,
            device_id: authorization.device_id,
            device_name: authorization.device_name.clone(),
            expires_at: Utc::now() + Duration::seconds(WRITER_LEASE_TTL_SECS as i64),
        };
        let snapshot = lease.snapshot();
        // A size pin belongs to the device that took it, so control changing hands drops it.
        // Without this a takeover would inherit the previous phone's claim on the pane's width.
        if self
            .remote_size_pins
            .get(&key)
            .is_some_and(|pinned_by| *pinned_by != authorization.device_id)
        {
            self.remote_size_pins.remove(&key);
        }
        self.writer_leases.insert(key, lease);
        Ok(snapshot)
    }

    /// Removes leases whose holder is gone. Holders that disconnect cleanly are handled by
    /// `session_disconnected` immediately; the TTL here is only a backstop for sessions that
    /// vanished without one. Clients learn about a pruned lease through the next workspace
    /// snapshot push, since `PaneSnapshot::writer_lease` participates in the change fingerprint.
    fn prune_writer_leases(&mut self, ctx: &mut ModelContext<Self>) {
        let now = Utc::now();
        let expired = self
            .writer_leases
            .iter()
            .filter(|(_, lease)| writer_lease_expired(&self.connected_sessions, lease, now))
            .map(|(target, _)| target.clone())
            .collect::<Vec<_>>();
        for target in expired {
            self.release_pane_control(&target);
            if let Some(resolved) = self.resolve_target_key(&target, ctx) {
                resolved.terminal.update(ctx, |terminal, ctx| {
                    terminal.remote_control_restore_desktop_size(ctx);
                });
            }
        }
    }

    /// Panes the Mac is itself showing right now: Clinch is the frontmost app, the pane lives in
    /// the active window, and its project and tab are the visible ones.
    ///
    /// A PTY carries exactly one `winsize`, so a pane cannot be one width for the Mac and another
    /// for a phone. These panes therefore belong to the person at the keyboard, and a remote
    /// viewport mirrors their width rather than imposing its own.
    fn desktop_watched_targets(&self, ctx: &mut ModelContext<Self>) -> HashSet<TargetKey> {
        let mut watched = HashSet::new();
        if !ctx.windows().app_is_active() {
            return watched;
        }
        let Some(active_window) = ctx.windows().active_window() else {
            return watched;
        };
        let Some(root) = ctx.root_view::<RootView>(active_window) else {
            return watched;
        };
        let Some(project_window_handle) = root.as_ref(ctx).project_window() else {
            return watched;
        };
        project_window_handle.read(ctx, |project_window, ctx| {
            let active_project_index = project_window.active_project_index();
            for (project_index, (project_id, workspace_handle)) in
                project_window.projects().enumerate()
            {
                if project_index != active_project_index {
                    continue;
                }
                let workspace = workspace_handle.as_ref(ctx);
                let active_tab_index = workspace.active_tab_index();
                for (tab_index, pane_group_handle) in workspace.tab_views().enumerate() {
                    if tab_index != active_tab_index {
                        continue;
                    }
                    // Every visible pane in the tab counts, not just the focused one: a split
                    // pane is equally on screen, and shrinking it would be just as disruptive.
                    for pane_id in pane_group_handle.as_ref(ctx).visible_pane_ids() {
                        watched.insert(TargetKey {
                            project_id: project_id.opaque_id(),
                            tab_id: pane_group_handle.id().to_string(),
                            pane_id: pane_opaque_id(pane_id),
                        });
                    }
                }
            }
        });
        watched
    }

    /// Hands a pane's width back to the Mac as soon as the Mac is showing it again. Desktop
    /// typing already preempts instantly through `preempt_writer_for_desktop_input`; this covers
    /// the case the user actually notices most — switching to the tab and simply *looking* at it,
    /// which previously left the pane stuck at phone dimensions until they typed.
    fn reclaim_desktop_sizes(
        &mut self,
        watched: &HashSet<TargetKey>,
        ctx: &mut ModelContext<Self>,
    ) {
        let reclaimable = watched
            .iter()
            .filter(|target| !self.remote_size_pins.contains_key(*target))
            .cloned()
            .collect::<Vec<_>>();
        for target in reclaimable {
            if let Some(resolved) = self.resolve_target_key(&target, ctx) {
                resolved.terminal.update(ctx, |terminal, ctx| {
                    // A no-op unless a remote viewport is actually driving this pane, so this
                    // never re-emits SIGWINCH for panes the Mac already sizes.
                    terminal.remote_control_restore_desktop_size(ctx);
                });
            }
        }
    }

    fn snapshot(&mut self, ctx: &mut ModelContext<Self>) -> WorkspaceSnapshot {
        self.prune_writer_leases(ctx);
        let desktop_watched = self.desktop_watched_targets(ctx);
        self.reclaim_desktop_sizes(&desktop_watched, ctx);
        self.refresh_recent_agent_sessions(ctx);
        let mut projects = Vec::new();
        let mut active_target = None;
        let mut order = 0u32;
        let mut topology = DefaultHasher::new();

        for window_id in ctx.window_ids() {
            let Some(root) = ctx.root_view::<RootView>(window_id) else {
                continue;
            };
            let Some(project_window_handle) = root.as_ref(ctx).project_window() else {
                continue;
            };
            project_window_handle.read(ctx, |project_window, ctx| {
                let active_project_index = project_window.active_project_index();
                for (project_index, (project_id, workspace_handle)) in
                    project_window.projects().enumerate()
                {
                    let project_id_string = project_id.opaque_id();
                    project_id_string.hash(&mut topology);
                    let project_active = project_index == active_project_index;
                    let workspace = workspace_handle.as_ref(ctx);
                    let active_tab_index = workspace.active_tab_index();
                    let project_title = workspace.project_display_name(ctx);
                    let mut tabs = Vec::new();

                    for (tab_index, pane_group_handle) in workspace.tab_views().enumerate() {
                        let pane_group = pane_group_handle.as_ref(ctx);
                        let tab_id = pane_group_handle.id().to_string();
                        tab_id.hash(&mut topology);
                        let section = workspace
                            .tabs
                            .get(tab_index)
                            .and_then(|tab| tab.group_id)
                            .and_then(|group_id| {
                                workspace.tab_groups.get(&group_id).map(|group| {
                                    (
                                        group_id.0.to_string(),
                                        group
                                            .name
                                            .clone()
                                            .unwrap_or_else(|| "New Section".to_owned()),
                                    )
                                })
                            });
                        section.hash(&mut topology);
                        let (section_id, section_name) = section
                            .map(|(id, name)| (Some(id), Some(name)))
                            .unwrap_or((None, None));
                        let tab_active = tab_index == active_tab_index;
                        let focused_pane = pane_group.focused_pane_id(ctx);
                        let tab_unread = {
                            let notifications = AgentNotificationsModel::as_ref(ctx).notifications();
                            pane_group.terminal_views(ctx).into_iter().any(|terminal| {
                                notifications.has_unread_for_terminal_view(terminal.id())
                            })
                        };
                        let mut panes = Vec::new();
                        let mut tab_activity = ProjectActivity::Idle;
                        let mut tab_kind = TabKind::Other;
                        let mut remote_host = None;

                        for pane_id in pane_group.visible_pane_ids() {
                            let pane_id_string = pane_opaque_id(pane_id);
                            pane_id_string.hash(&mut topology);
                            let pane_active = pane_id == focused_pane;
                            let target = TargetRef {
                                app_instance_id: self.app_instance_id,
                                project_id: project_id_string.clone(),
                                tab_id: tab_id.clone(),
                                pane_id: pane_id_string.clone(),
                            };
                            let terminal = pane_group.terminal_view_from_pane_id(pane_id, ctx);
                            let (pane_kind, activity, agent_state, cwd, dimensions, quick_inserts) =
                                if let Some(terminal) = terminal {
                                    let terminal_ref = terminal.as_ref(ctx);
                                    let session =
                                        CLIAgentSessionsModel::as_ref(ctx).session(terminal.id());
                                    if remote_host.is_none() {
                                        remote_host = terminal_ref.remote_control_remote_host(ctx).or_else(|| {
                                            session.and_then(|session| session.remote_host.clone())
                                        });
                                    }
                                    let pane_kind = pane_kind(session.map(|session| session.agent));
                                    let notifications =
                                        AgentNotificationsModel::as_ref(ctx).notifications();
                                    let activity = pane_activity(
                                        session,
                                        terminal_ref.current_state().state,
                                        notifications
                                            .has_unread_completed_project_cli_agent_for_terminal_view(
                                                terminal.id(),
                                            ),
                                        notifications
                                            .has_other_unread_project_activity_for_terminal_view(
                                                terminal.id(),
                                            ),
                                    );
                                    let agent_state = session.map(agent_state);
                                    let cwd = terminal_ref
                                        .pwd_if_local(ctx)
                                        .or_else(|| terminal_ref.pwd());
                                    let dimensions = terminal_ref
                                        .remote_control_is_ready()
                                        .then(|| {
                                            let (columns, rows) =
                                                terminal_ref.remote_control_dimensions();
                                            TerminalDimensions { columns, rows }
                                        });
                                    let quick_inserts = self
                                        .quick_inserts_for_terminal(&terminal, ctx)
                                        .into_iter()
                                        .map(|item| item.descriptor)
                                        .collect();
                                    (
                                        pane_kind,
                                        activity,
                                        agent_state,
                                        cwd,
                                        dimensions,
                                        quick_inserts,
                                    )
                                } else {
                                    (
                                        PaneKind::Other,
                                        ProjectActivity::Idle,
                                        None,
                                        None,
                                        None,
                                        Vec::new(),
                                    )
                                };
                            tab_kind = merge_tab_kind(tab_kind, &pane_kind);
                            tab_activity = merge_activity(tab_activity, activity.clone());
                            if project_active && tab_active && pane_active && dimensions.is_some() {
                                active_target = Some(target.clone());
                            }
                            let target_key = TargetKey::from(&target);
                            panes.push(PaneSnapshot {
                                id: pane_id_string,
                                title: pane_group
                                    .pane_title(pane_id, ctx)
                                    .filter(|title| !title.trim().is_empty())
                                    .unwrap_or_else(|| "Terminal".to_owned()),
                                kind: pane_kind,
                                cwd,
                                active: pane_active,
                                agent_state,
                                dimensions,
                                writer_lease: self
                                    .writer_leases
                                    .get(&target_key)
                                    .map(WriterLease::snapshot),
                                quick_inserts,
                                desktop_watching: desktop_watched.contains(&target_key),
                                size_pinned_by: self.remote_size_pins.get(&target_key).copied(),
                            });
                        }
                        tabs.push(TabSnapshot {
                            id: tab_id,
                            title: nonempty_title(pane_group.display_title(ctx), "New tab"),
                            section_id,
                            section_name,
                            kind: tab_kind,
                            active: tab_active,
                            activity: tab_activity,
                            unread: tab_unread,
                            remote_host,
                            panes,
                        });
                    }

                    let project_activity =
                        tabs.iter().fold(ProjectActivity::Idle, |activity, tab| {
                            merge_activity(activity, tab.activity.clone())
                        });
                    let agent_counts = workspace.project_cli_agent_counts(ctx);
                    let tasks = workspace
                        .workspace_tasks()
                        .iter()
                        .map(|task| {
                            task.id.hash(&mut topology);
                            task.text.hash(&mut topology);
                            TaskSnapshot {
                                id: TaskId(task.id.0),
                                text: task.text.clone(),
                            }
                        })
                        .collect();
                    projects.push(ProjectSnapshot {
                        id: project_id_string,
                        title: nonempty_title(project_title, "New project"),
                        order,
                        active: project_active,
                        activity: project_activity,
                        badges: ProjectBadgeSnapshot {
                            has_other_unread: workspace.has_other_unread_project_activity(ctx),
                            done: u32::try_from(agent_counts.done).unwrap_or(u32::MAX),
                            working: u32::try_from(agent_counts.working).unwrap_or(u32::MAX),
                            running_commands: u32::try_from(agent_counts.running_commands)
                                .unwrap_or(u32::MAX),
                        },
                        tabs,
                        tasks,
                    });
                    order = order.saturating_add(1);
                }
            });
        }

        let fingerprint = topology.finish();
        if let Some(previous) = self.last_topology_fingerprint {
            if previous != fingerprint {
                self.revision = next_workspace_revision(self.revision);
            }
        }
        self.last_topology_fingerprint = Some(fingerprint);

        WorkspaceSnapshot {
            revision: self.revision,
            sequence: self.sequence,
            host: HostSnapshot {
                app_instance_id: self.app_instance_id,
                name: gethostname::gethostname().to_string_lossy().into_owned(),
                // Tailscale Serve does not expose whether the current packet took a peer-to-peer
                // path or a DERP relay, so do not claim one without evidence.
                connection_path: ConnectionPath::Unknown,
                capabilities: vec![
                    Capability::View,
                    Capability::Control,
                    Capability::CreateSession,
                    Capability::Upload,
                ],
            },
            projects,
            active_target,
            usage: usage_snapshots(ctx),
            recent_agent_sessions: self.recent_agent_sessions.clone(),
            paired_devices: self.pairing.paired_devices(Utc::now()).unwrap_or_default(),
        }
    }

    fn refresh_recent_agent_sessions(&mut self, ctx: &mut ModelContext<Self>) {
        if self.recent_agent_sessions_refresh_in_flight
            || self
                .recent_agent_sessions_refreshed_at
                .is_some_and(|last_refresh| last_refresh.elapsed() < StdDuration::from_secs(30))
        {
            return;
        }
        self.recent_agent_sessions_refresh_in_flight = true;
        ctx.spawn(
            async move { recent_agent_session_snapshots() },
            |adapter, sessions, ctx| {
                adapter.recent_agent_sessions = sessions;
                adapter.recent_agent_sessions_refreshed_at = Some(Instant::now());
                adapter.recent_agent_sessions_refresh_in_flight = false;
                ctx.notify();
            },
        );
    }

    fn quick_inserts_for_terminal(
        &self,
        terminal: &ViewHandle<TerminalView>,
        ctx: &AppContext,
    ) -> Vec<ResolvedQuickInsert> {
        let has_cli_agent = CLIAgentSessionsModel::as_ref(ctx)
            .session(terminal.id())
            .is_some();
        let items = if has_cli_agent {
            SessionSettings::as_ref(ctx)
                .cli_agent_footer_chip_selection
                .all_items()
        } else {
            SessionSettings::as_ref(ctx)
                .terminal_footer_chip_selection
                .all_items()
        };
        let mut config_hasher = DefaultHasher::new();
        items.hash(&mut config_hasher);
        // This value crosses JSON twice (Rust -> browser -> Rust). Keep it in JavaScript's exact
        // integer range or every button will look stale after `JSON.parse` rounds the hash.
        let configuration_revision = javascript_safe_integer(config_hasher.finish());

        items
            .into_iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let (label, text, kind) = match item {
                    AgentToolbarItemKind::CustomInsert { label, text, .. } => {
                        (label, text, QuickInsertKind::Custom)
                    }
                    AgentToolbarItemKind::Compact if has_cli_agent => (
                        "Compact".to_owned(),
                        "/compact".to_owned(),
                        QuickInsertKind::BuiltIn,
                    ),
                    AgentToolbarItemKind::ContinuePrompt if has_cli_agent => (
                        "Continue".to_owned(),
                        "Continue".to_owned(),
                        QuickInsertKind::BuiltIn,
                    ),
                    AgentToolbarItemKind::LooksGoodPrompt if has_cli_agent => (
                        "LGTM".to_owned(),
                        "Looks good to me, continue".to_owned(),
                        QuickInsertKind::BuiltIn,
                    ),
                    _ => return None,
                };
                if text.trim().is_empty() {
                    return None;
                }
                let mut id_hasher = DefaultHasher::new();
                self.quick_insert_salt.hash(&mut id_hasher);
                configuration_revision.hash(&mut id_hasher);
                index.hash(&mut id_hasher);
                text.hash(&mut id_hasher);
                Some(ResolvedQuickInsert {
                    descriptor: QuickInsertDescriptor {
                        id: format!("qi-{:016x}", id_hasher.finish()),
                        configuration_revision,
                        label: nonempty_title(label, "Quick insert"),
                        kind,
                        // The phone defaults to preview. A local phone preference may explicitly
                        // choose one-tap submission, but the descriptor never overrides that.
                        submits_immediately: false,
                    },
                    text,
                })
            })
            .collect()
    }

    fn resolve_quick_insert(
        &self,
        terminal: &ViewHandle<TerminalView>,
        item_id: &str,
        configuration_revision: u64,
        ctx: &AppContext,
    ) -> Option<ResolvedQuickInsert> {
        self.quick_inserts_for_terminal(terminal, ctx)
            .into_iter()
            .find(|item| {
                item.descriptor.id == item_id
                    && item.descriptor.configuration_revision == configuration_revision
            })
    }

    fn response(
        &mut self,
        request_id: Option<RequestId>,
        payload: ServerMessage,
    ) -> ServerEnvelope {
        self.sequence = self.sequence.saturating_add(1);
        ServerEnvelope {
            version: PROTOCOL_VERSION,
            request_id,
            sequence: Some(self.sequence),
            payload,
        }
    }

    fn command_accepted(&mut self, request_id: RequestId) -> ServerEnvelope {
        self.response(
            Some(request_id),
            ServerMessage::CommandAccepted {
                workspace_revision: self.revision,
            },
        )
    }

    fn target_error(
        &mut self,
        request_id: RequestId,
        (code, message, retryable): (ProtocolErrorCode, String, bool),
    ) -> ServerEnvelope {
        self.error(Some(request_id), code, message, retryable)
    }

    fn error(
        &mut self,
        request_id: Option<RequestId>,
        code: ProtocolErrorCode,
        message: String,
        retryable: bool,
    ) -> ServerEnvelope {
        let current_revision = self.revision;
        self.response(
            request_id,
            ServerMessage::Error(ProtocolError {
                code,
                message,
                retryable,
                current_revision: Some(current_revision),
            }),
        )
    }

    fn bump_topology_revision(&mut self) {
        self.revision = next_workspace_revision(self.revision);
        self.last_topology_fingerprint = None;
    }

    fn cached(&self, session_id: AuthSessionId, request_id: RequestId) -> Option<ServerEnvelope> {
        self.idempotency
            .get(&session_id)
            .and_then(|entries| entries.iter().find(|(id, _)| *id == request_id))
            .map(|(_, response)| response.clone())
    }

    fn remember(
        &mut self,
        session_id: AuthSessionId,
        request_id: RequestId,
        response: ServerEnvelope,
    ) {
        let entries = self.idempotency.entry(session_id).or_default();
        while entries.len() >= MAX_IDEMPOTENCY_RESULTS_PER_SESSION {
            entries.pop_front();
        }
        entries.push_back((request_id, response));
    }
}

fn initial_workspace_revision(random: u64) -> u64 {
    javascript_safe_integer(random).max(1)
}

fn next_workspace_revision(current: u64) -> u64 {
    if current >= MAX_JAVASCRIPT_SAFE_INTEGER {
        1
    } else {
        current + 1
    }
}

fn pane_opaque_id(pane_id: PaneId) -> String {
    format!(
        "pane_{}",
        URL_SAFE_NO_PAD.encode(pane_id.to_string().as_bytes())
    )
}

fn optional_local_directory(path: Option<String>) -> Result<Option<PathBuf>, String> {
    path.map(|path| canonical_local_directory(&path))
        .transpose()
}

fn canonical_local_directory(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err("The working directory must be an absolute local path.".to_owned());
    }
    let canonical = std::fs::canonicalize(&path)
        .map_err(|_| "The working directory does not exist or cannot be accessed.".to_owned())?;
    if !canonical.is_dir() {
        return Err("The working directory must be a directory.".to_owned());
    }
    Ok(canonical)
}

fn nonempty_title(value: String, fallback: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        fallback.to_owned()
    } else {
        collapsed.chars().take(120).collect()
    }
}

fn pane_kind(agent: Option<CLIAgent>) -> PaneKind {
    match agent {
        Some(CLIAgent::Claude) => PaneKind::ClaudeCode,
        Some(CLIAgent::Codex) => PaneKind::Codex,
        Some(_) => PaneKind::Other,
        None => PaneKind::Terminal,
    }
}

fn merge_tab_kind(current: TabKind, pane: &PaneKind) -> TabKind {
    match pane {
        PaneKind::ClaudeCode => TabKind::ClaudeCode,
        PaneKind::Codex if current != TabKind::ClaudeCode => TabKind::Codex,
        PaneKind::Terminal if current == TabKind::Other => TabKind::Terminal,
        PaneKind::Notebook if current == TabKind::Other => TabKind::Notebook,
        _ => current,
    }
}

fn agent_state(session: &crate::terminal::cli_agent_sessions::CLIAgentSession) -> AgentState {
    if session.session_context.stop_reason.is_some() {
        return AgentState::RateLimited;
    }
    match &session.status {
        CLIAgentSessionStatus::Blocked { .. } => AgentState::NeedsAttention,
        CLIAgentSessionStatus::Success => AgentState::Done,
        CLIAgentSessionStatus::InProgress if session.is_actively_working() => AgentState::Working,
        CLIAgentSessionStatus::InProgress => AgentState::Idle,
    }
}

fn pane_activity(
    session: Option<&crate::terminal::cli_agent_sessions::CLIAgentSession>,
    terminal_state: TerminalViewState,
    has_unread_completed: bool,
    has_other_unread: bool,
) -> ProjectActivity {
    if let Some(session) = session {
        return if session.is_actively_working() {
            ProjectActivity::Working
        } else if matches!(session.status, CLIAgentSessionStatus::Blocked { .. })
            || session.session_context.stop_reason.is_some()
            || has_other_unread
        {
            ProjectActivity::NeedsAttention
        } else if session.turn_interrupted_by_user || has_unread_completed {
            ProjectActivity::Done
        } else {
            ProjectActivity::Idle
        };
    }
    if terminal_state == TerminalViewState::LongRunning {
        ProjectActivity::RunningCommand
    } else {
        ProjectActivity::Idle
    }
}

fn activity_priority(activity: &ProjectActivity) -> u8 {
    match activity {
        ProjectActivity::NeedsAttention => 5,
        ProjectActivity::Working => 4,
        ProjectActivity::RunningCommand => 3,
        ProjectActivity::Done => 2,
        ProjectActivity::Idle => 1,
    }
}

fn merge_activity(left: ProjectActivity, right: ProjectActivity) -> ProjectActivity {
    if activity_priority(&right) > activity_priority(&left) {
        right
    } else {
        left
    }
}

fn recent_agent_session_snapshots() -> Vec<RecentAgentSessionSnapshot> {
    crate::agent_resume::recent_conversations(50)
        .into_iter()
        .filter_map(|conversation| {
            if conversation.session_id.len() > MAX_OPAQUE_ID_BYTES
                || !conversation
                    .session_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                return None;
            }
            let provider = match AgentResumeProvider::from_agent_name(&conversation.agent)? {
                AgentResumeProvider::Claude => AgentProvider::ClaudeCode,
                AgentResumeProvider::Codex => AgentProvider::Codex,
            };
            let title = conversation
                .first_prompt
                .map(|prompt| nonempty_title(prompt, "Agent conversation"))
                .unwrap_or_else(|| format!("{} conversation", conversation.agent));
            let started_at = chrono::DateTime::parse_from_rfc3339(&conversation.start_ts)
                .ok()
                .map(|timestamp| timestamp.with_timezone(&Utc));
            Some(RecentAgentSessionSnapshot {
                durable_session_id: conversation.session_id,
                provider,
                title,
                cwd: conversation.cwd.filter(|cwd| {
                    cwd.len() <= MAX_PATH_BYTES && !cwd.chars().any(char::is_control)
                }),
                started_at,
            })
        })
        .collect()
}

fn usage_snapshots(ctx: &ModelContext<WorkspaceAdapter>) -> Vec<UsageSnapshot> {
    let usage_model = CliAgentUsageModel::as_ref(ctx);
    let latest = usage_model.latest();
    let updated_at = usage_model.last_updated_at();
    let live_plan_gauges_enabled = *CliAgentUsageSettings::as_ref(ctx).show_plan_limits;
    [
        (AgentProvider::ClaudeCode, &latest.claude),
        (AgentProvider::Codex, &latest.codex),
    ]
    .into_iter()
    .map(|(provider, usage)| {
        let limit_windows = usage
            .plan
            .into_iter()
            .flat_map(|plan| {
                [
                    ("5-hour", plan.session),
                    ("Weekly", plan.weekly),
                    ("Fable weekly", plan.fable_weekly),
                ]
            })
            .filter_map(|(label, window)| {
                window.map(|window| UsageLimitWindowSnapshot {
                    label: label.to_owned(),
                    used_percent: window.percent.clamp(0.0, 100.0),
                    resets_at: window.resets_at,
                })
            })
            .collect();
        let token_windows = [
            ("Session", &usage.session),
            ("Today", &usage.today),
            ("This week", &usage.week),
            ("This month", &usage.month),
        ]
        .into_iter()
        .map(|(label, window)| UsageTokenWindowSnapshot {
            label: label.to_owned(),
            input_tokens: window.tokens.input,
            output_tokens: window.tokens.output,
            cache_read_tokens: window.tokens.cache_read,
            cache_write_tokens: window.tokens.cache_write,
            estimated_cost_usd: window.cost_usd.max(0.0),
        })
        .collect();
        let selected_window = usage.plan.and_then(|plan| {
            [plan.session, plan.weekly]
                .into_iter()
                .flatten()
                .max_by(|a, b| {
                    a.percent
                        .partial_cmp(&b.percent)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });
        let has_local_usage = usage.session.tokens.total() > 0
            || usage.today.tokens.total() > 0
            || usage.week.tokens.total() > 0
            || usage.month.tokens.total() > 0;
        UsageSnapshot {
            provider,
            state: if selected_window.is_some() || has_local_usage {
                if usage.plan_needs_authorization {
                    UsageState::Stale
                } else {
                    UsageState::Available
                }
            } else {
                UsageState::Unavailable
            },
            updated_at,
            reset_at: selected_window.and_then(|window| window.resets_at),
            used_percent: selected_window.map(|window| window.percent.clamp(0.0, 100.0)),
            model: None,
            limit_windows,
            token_windows,
            source: "Latest local Clinch usage snapshot".to_owned(),
            live_plan_gauges_enabled_on_mac: live_plan_gauges_enabled,
        }
    })
    .collect()
}

fn required_capability(message: &ClientMessage) -> Option<Capability> {
    match message {
        ClientMessage::Ping | ClientMessage::RequestSnapshot | ClientMessage::SelectTarget(_) => {
            Some(Capability::View)
        }
        ClientMessage::AcquireWriterLease(_)
        | ClientMessage::ReleaseWriterLease(_)
        | ClientMessage::SubmitComposerText(_)
        | ClientMessage::RawTerminalInput(_)
        | ClientMessage::InterruptTerminal(_)
        | ClientMessage::TerminalResize(_)
        | ClientMessage::SetTerminalSizePin(_)
        | ClientMessage::TerminalKey(_)
        | ClientMessage::QuickInsertPreview(_)
        | ClientMessage::QuickInsertSubmit(_)
        | ClientMessage::CreateTask(_)
        | ClientMessage::DeleteTask(_) => Some(Capability::Control),
        ClientMessage::CreateProject(_)
        | ClientMessage::CreateSession(_)
        | ClientMessage::ResumeSession(_)
        | ClientMessage::LaunchTask(_) => Some(Capability::CreateSession),
        ClientMessage::UploadBegin(_)
        | ClientMessage::UploadCommit(_)
        | ClientMessage::UploadCancel(_) => Some(Capability::Upload),
        // Unpairing needs no capability beyond being the authenticated device itself, and it
        // never reaches the adapter: the gateway answers it directly.
        ClientMessage::UnpairDevice | ClientMessage::Disconnect => None,
    }
}

fn is_idempotent_mutation(message: &ClientMessage) -> bool {
    matches!(
        message,
        ClientMessage::AcquireWriterLease(_)
            | ClientMessage::ReleaseWriterLease(_)
            | ClientMessage::SubmitComposerText(_)
            | ClientMessage::RawTerminalInput(_)
            | ClientMessage::InterruptTerminal(_)
            | ClientMessage::TerminalResize(_)
            | ClientMessage::SetTerminalSizePin(_)
            | ClientMessage::TerminalKey(_)
            | ClientMessage::CreateProject(_)
            | ClientMessage::CreateSession(_)
            | ClientMessage::ResumeSession(_)
            | ClientMessage::CreateTask(_)
            | ClientMessage::DeleteTask(_)
            | ClientMessage::LaunchTask(_)
            | ClientMessage::QuickInsertSubmit(_)
    )
}

#[cfg(test)]
#[path = "workspace_adapter_tests.rs"]
mod tests;

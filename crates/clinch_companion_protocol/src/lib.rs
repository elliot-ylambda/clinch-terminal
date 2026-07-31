//! Transport-neutral messages and validation for Clinch Remote Control.
//!
//! This crate intentionally knows nothing about WarpUI, Tailscale, Axum, or terminal models. The
//! native gateway and the browser client share this contract, while each transport remains free to
//! choose its own framing.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;
use uuid::Uuid;

pub const PROTOCOL_VERSION: u16 = 1;
pub const SUPPORTED_PROTOCOL_VERSIONS: &[u16] = &[PROTOCOL_VERSION];

/// Largest integer JavaScript can represent exactly in a JSON `number`.
///
/// Every protocol integer that a browser must send back to Clinch must stay at or below this
/// value. Otherwise `JSON.parse` rounds it before the client can echo it back for exact revision
/// checks.
pub const MAX_JAVASCRIPT_SAFE_INTEGER: u64 = (1u64 << 53) - 1;

pub const fn javascript_safe_integer(value: u64) -> u64 {
    value & MAX_JAVASCRIPT_SAFE_INTEGER
}

pub const PAIRING_INVITATION_TTL_SECS: u64 = 5 * 60;
pub const AUTH_CHALLENGE_TTL_SECS: u64 = 60;
pub const AUTH_SESSION_TTL_SECS: u64 = 15 * 60;
pub const DEVICE_INACTIVITY_LIMIT_DAYS: i64 = 90;
pub const WRITER_LEASE_TTL_SECS: u64 = 30;

pub const MAX_JSON_MESSAGE_BYTES: usize = 256 * 1024;
pub const MAX_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_PROMPT_BYTES: usize = 256 * 1024;
// Base64 expands this into the JSON TerminalSnapshot envelope, so keep the raw scrollback below
// the control-message ceiling and stream all subsequent PTY data as binary frames.
pub const MAX_TERMINAL_SNAPSHOT_BYTES: usize = 128 * 1024;
pub const MAX_UPLOAD_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_UPLOAD_CHUNK_BYTES: usize = 256 * 1024;
pub const MAX_TERMINAL_FRAME_BYTES: usize = 256 * 1024;
pub const MAX_DEVICE_NAME_BYTES: usize = 96;
pub const MAX_PATH_BYTES: usize = 4096;
pub const MAX_FILENAME_BYTES: usize = 255;
pub const MAX_MIME_BYTES: usize = 128;
pub const MAX_OPAQUE_ID_BYTES: usize = 256;
pub const MAX_REPLAY_EVENTS: usize = 4096;
pub const MAX_CONNECTIONS_PER_DEVICE: usize = 3;
pub const MAX_IDEMPOTENCY_RESULTS_PER_SESSION: usize = 1024;

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            Deserialize,
            Eq,
            Hash,
            JsonSchema,
            Ord,
            PartialEq,
            PartialOrd,
            Serialize,
            TS,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

uuid_id!(RequestId);
uuid_id!(AppInstanceId);
uuid_id!(PairingInvitationId);
uuid_id!(PairingClaimId);
uuid_id!(DeviceId);
uuid_id!(ChallengeId);
uuid_id!(AuthSessionId);
uuid_id!(UploadId);
uuid_id!(TerminalStreamId);

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    View,
    Control,
    CreateSession,
    Upload,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DevicePlatform {
    Ios,
    Ipados,
    Macos,
    Android,
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Connected,
    Reconnecting,
    MacOffline,
    TailscaleNeeded,
    AuthorizationRevoked,
    VersionIncompatible,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionPath {
    TailnetDirect,
    TailnetRelay,
    LoopbackDevelopment,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProjectActivity {
    Working,
    Done,
    NeedsAttention,
    Idle,
    RunningCommand,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TabKind {
    Terminal,
    ClaudeCode,
    Codex,
    Notebook,
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PaneKind {
    Terminal,
    ClaudeCode,
    Codex,
    Notebook,
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Working,
    Done,
    NeedsAttention,
    Idle,
    RateLimited,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AgentProvider {
    ClaudeCode,
    Codex,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum UsageState {
    Available,
    Stale,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum QuickInsertKind {
    BuiltIn,
    Custom,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Terminal,
    ClaudeCode,
    Codex,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TerminalKey {
    Escape,
    Tab,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ControlD,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct TargetRef {
    pub app_instance_id: AppInstanceId,
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
}

impl TargetRef {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_opaque_id("project_id", &self.project_id)?;
        validate_opaque_id("tab_id", &self.tab_id)?;
        validate_opaque_id("pane_id", &self.pane_id)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct TerminalDimensions {
    pub columns: u16,
    pub rows: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct QuickInsertDescriptor {
    pub id: String,
    #[ts(type = "number")]
    pub configuration_revision: u64,
    pub label: String,
    pub kind: QuickInsertKind,
    pub submits_immediately: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct WriterLeaseSnapshot {
    pub device_id: DeviceId,
    pub device_name: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct PaneSnapshot {
    pub id: String,
    pub title: String,
    pub kind: PaneKind,
    pub cwd: Option<String>,
    pub active: bool,
    pub agent_state: Option<AgentState>,
    pub dimensions: Option<TerminalDimensions>,
    pub writer_lease: Option<WriterLeaseSnapshot>,
    pub quick_inserts: Vec<QuickInsertDescriptor>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct TabSnapshot {
    pub id: String,
    pub title: String,
    pub kind: TabKind,
    pub active: bool,
    pub activity: ProjectActivity,
    pub unread: bool,
    pub remote_host: Option<String>,
    pub panes: Vec<PaneSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct ProjectSnapshot {
    pub id: String,
    pub title: String,
    pub order: u32,
    pub active: bool,
    pub activity: ProjectActivity,
    pub tabs: Vec<TabSnapshot>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize, TS)]
pub struct UsageLimitWindowSnapshot {
    pub label: String,
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize, TS)]
pub struct UsageTokenWindowSnapshot {
    pub label: String,
    #[ts(type = "number")]
    pub input_tokens: u64,
    #[ts(type = "number")]
    pub output_tokens: u64,
    #[ts(type = "number")]
    pub cache_read_tokens: u64,
    #[ts(type = "number")]
    pub cache_write_tokens: u64,
    pub estimated_cost_usd: f64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize, TS)]
pub struct UsageSnapshot {
    pub provider: AgentProvider,
    pub state: UsageState,
    pub updated_at: Option<DateTime<Utc>>,
    pub reset_at: Option<DateTime<Utc>>,
    pub used_percent: Option<f64>,
    pub model: Option<String>,
    pub limit_windows: Vec<UsageLimitWindowSnapshot>,
    pub token_windows: Vec<UsageTokenWindowSnapshot>,
    pub source: String,
    pub live_plan_gauges_enabled_on_mac: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct RecentAgentSessionSnapshot {
    pub durable_session_id: String,
    pub provider: AgentProvider,
    pub title: String,
    pub cwd: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct DeviceSummary {
    pub id: DeviceId,
    pub name: String,
    pub platform: DevicePlatform,
    pub capabilities: Vec<Capability>,
    pub connected: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct HostSnapshot {
    pub app_instance_id: AppInstanceId,
    pub name: String,
    pub connection_path: ConnectionPath,
    pub capabilities: Vec<Capability>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize, TS)]
pub struct WorkspaceSnapshot {
    #[ts(type = "number")]
    pub revision: u64,
    #[ts(type = "number")]
    pub sequence: u64,
    pub host: HostSnapshot,
    pub projects: Vec<ProjectSnapshot>,
    pub active_target: Option<TargetRef>,
    pub usage: Vec<UsageSnapshot>,
    pub recent_agent_sessions: Vec<RecentAgentSessionSnapshot>,
    pub paired_devices: Vec<DeviceSummary>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct TerminalSnapshot {
    pub target: TargetRef,
    pub stream_id: TerminalStreamId,
    #[ts(type = "number")]
    pub workspace_revision: u64,
    #[ts(type = "number")]
    pub terminal_sequence: u64,
    /// Base64-encoded raw PTY bytes. Subsequent bytes use binary terminal-output frames.
    pub data_base64: String,
    pub dimensions: TerminalDimensions,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize, TS)]
pub struct WorkspaceChanged {
    pub snapshot: WorkspaceSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct PairingInvitation {
    pub id: PairingInvitationId,
    pub pairing_url: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct PairingClaimRequest {
    pub invitation_id: PairingInvitationId,
    pub secret: String,
    pub device_name: String,
    pub platform: DevicePlatform,
    /// Base64-encoded 65-byte uncompressed SEC1 point for an ECDSA P-256 public key.
    pub public_key_p256_raw: String,
}

impl PairingClaimRequest {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_text("device_name", &self.device_name, 1, MAX_DEVICE_NAME_BYTES)?;
        validate_text("secret", &self.secret, 32, 512)?;
        let key = BASE64_STANDARD
            .decode(&self.public_key_p256_raw)
            .map_err(|_| ProtocolValidationError::InvalidP256PublicKey)?;
        if key.len() != 65 || key.first() != Some(&4) {
            return Err(ProtocolValidationError::InvalidP256PublicKey);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct PairingClaimReceipt {
    pub claim_id: PairingClaimId,
    pub claim_secret: String,
    pub device_name: String,
    pub public_key_fingerprint: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct PairingStatusRequest {
    pub claim_id: PairingClaimId,
    pub claim_secret: String,
}

impl PairingStatusRequest {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_text("claim_secret", &self.claim_secret, 32, 512)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub enum PairingStatus {
    Pending,
    Approved {
        device_id: DeviceId,
        capabilities: Vec<Capability>,
    },
    Rejected,
    Expired,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct AuthChallengeRequest {
    pub device_id: DeviceId,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct AuthChallenge {
    pub id: ChallengeId,
    pub device_id: DeviceId,
    pub challenge: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct Authenticate {
    pub device_id: DeviceId,
    pub challenge_id: ChallengeId,
    /// Base64-encoded IEEE P1363 ECDSA P-256 signature (r || s).
    pub signature: String,
    #[ts(type = "number")]
    pub last_seen_sequence: u64,
}

impl Authenticate {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        let signature = BASE64_STANDARD
            .decode(&self.signature)
            .map_err(|_| ProtocolValidationError::InvalidP256Signature)?;
        if signature.len() != 64 {
            return Err(ProtocolValidationError::InvalidP256Signature);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct Authenticated {
    pub session_id: AuthSessionId,
    pub device: DeviceSummary,
    pub expires_at: DateTime<Utc>,
    #[ts(type = "number | null")]
    pub replayed_from_sequence: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct SelectTarget {
    pub target: TargetRef,
    #[ts(type = "number")]
    pub workspace_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct AcquireWriterLease {
    pub target: TargetRef,
    #[ts(type = "number")]
    pub workspace_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct ReleaseWriterLease {
    pub target: TargetRef,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct SubmitComposerText {
    pub target: TargetRef,
    #[ts(type = "number")]
    pub workspace_revision: u64,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct RawTerminalInput {
    pub target: TargetRef,
    #[ts(type = "number")]
    pub workspace_revision: u64,
    /// Base64-encoded PTY bytes for an interactive terminal/TUI.
    pub data_base64: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct InterruptTerminal {
    pub target: TargetRef,
    #[ts(type = "number")]
    pub workspace_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct TerminalResize {
    pub target: TargetRef,
    #[ts(type = "number")]
    pub workspace_revision: u64,
    pub dimensions: TerminalDimensions,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct TerminalKeyInput {
    pub target: TargetRef,
    #[ts(type = "number")]
    pub workspace_revision: u64,
    pub key: TerminalKey,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct CreateSession {
    pub app_instance_id: AppInstanceId,
    #[ts(type = "number")]
    pub workspace_revision: u64,
    pub project_id: String,
    pub kind: SessionKind,
    /// An exact local directory, or `None` to use Clinch's normal new-tab directory.
    pub cwd: Option<String>,
    pub initial_prompt: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct CreateProject {
    pub app_instance_id: AppInstanceId,
    #[ts(type = "number")]
    pub workspace_revision: u64,
    /// The currently selected project anchors the new project to the same native window.
    pub project_id: String,
    /// An exact local directory for the first terminal, or `None` for Clinch's default.
    pub cwd: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct ResumeSession {
    pub app_instance_id: AppInstanceId,
    #[ts(type = "number")]
    pub workspace_revision: u64,
    pub project_id: String,
    pub provider: AgentProvider,
    pub durable_session_id: String,
    pub cwd: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct QuickInsertPreviewRequest {
    pub target: TargetRef,
    #[ts(type = "number")]
    pub workspace_revision: u64,
    pub item_id: String,
    #[ts(type = "number")]
    pub configuration_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct QuickInsertPreview {
    pub item_id: String,
    #[ts(type = "number")]
    pub configuration_revision: u64,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct QuickInsertSubmit {
    pub target: TargetRef,
    #[ts(type = "number")]
    pub workspace_revision: u64,
    pub item_id: String,
    #[ts(type = "number")]
    pub configuration_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct UploadBegin {
    pub target: TargetRef,
    #[ts(type = "number")]
    pub workspace_revision: u64,
    pub filename: String,
    pub mime: String,
    #[ts(type = "number")]
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct UploadReady {
    pub upload_id: UploadId,
    pub chunk_size: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct UploadProgress {
    pub upload_id: UploadId,
    #[ts(type = "number")]
    pub received: u64,
    #[ts(type = "number")]
    pub total: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct UploadCommit {
    pub upload_id: UploadId,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct UploadCancel {
    pub upload_id: UploadId,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct UploadCompleted {
    pub upload_id: UploadId,
    pub inserted_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ClientMessage {
    Ping,
    RequestSnapshot,
    SelectTarget(SelectTarget),
    AcquireWriterLease(AcquireWriterLease),
    ReleaseWriterLease(ReleaseWriterLease),
    SubmitComposerText(SubmitComposerText),
    RawTerminalInput(RawTerminalInput),
    InterruptTerminal(InterruptTerminal),
    TerminalResize(TerminalResize),
    TerminalKey(TerminalKeyInput),
    CreateProject(CreateProject),
    CreateSession(CreateSession),
    ResumeSession(ResumeSession),
    QuickInsertPreview(QuickInsertPreviewRequest),
    QuickInsertSubmit(QuickInsertSubmit),
    UploadBegin(UploadBegin),
    UploadCommit(UploadCommit),
    UploadCancel(UploadCancel),
    Disconnect,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolErrorCode {
    Unauthorized,
    CapabilityDenied,
    InvalidRequest,
    UnsupportedVersion,
    TargetGone,
    RevisionConflict,
    WriterLeaseHeld,
    StaleQuickInsert,
    UploadRejected,
    RateLimited,
    ResyncRequired,
    Internal,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct ProtocolError {
    pub code: ProtocolErrorCode,
    pub message: String,
    pub retryable: bool,
    #[ts(type = "number")]
    pub current_revision: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize, TS)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ServerMessage {
    Hello {
        supported_versions: Vec<u16>,
        host_name: String,
    },
    Authenticated(Authenticated),
    Pong,
    Snapshot(WorkspaceSnapshot),
    WorkspaceChanged(WorkspaceChanged),
    TerminalSnapshot(TerminalSnapshot),
    TerminalStreamClosed {
        stream_id: TerminalStreamId,
        reason: String,
    },
    WriterLeaseChanged {
        target: TargetRef,
        lease: Option<WriterLeaseSnapshot>,
    },
    QuickInsertPreview(QuickInsertPreview),
    UploadReady(UploadReady),
    UploadProgress(UploadProgress),
    UploadCompleted(UploadCompleted),
    CommandAccepted {
        #[ts(type = "number")]
        workspace_revision: u64,
    },
    ConnectionState(ConnectionState),
    Error(ProtocolError),
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct ClientEnvelope {
    pub version: u16,
    pub request_id: RequestId,
    pub payload: ClientMessage,
}

impl ClientEnvelope {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        if self.version != PROTOCOL_VERSION {
            return Err(ProtocolValidationError::UnsupportedVersion(self.version));
        }

        match &self.payload {
            ClientMessage::SelectTarget(message) => message.target.validate()?,
            ClientMessage::AcquireWriterLease(message) => message.target.validate()?,
            ClientMessage::ReleaseWriterLease(message) => message.target.validate()?,
            ClientMessage::SubmitComposerText(message) => {
                message.target.validate()?;
                validate_text("composer_text", &message.text, 1, MAX_PROMPT_BYTES)?;
            }
            ClientMessage::RawTerminalInput(message) => {
                message.target.validate()?;
                validate_base64_payload(
                    "raw_terminal_input",
                    &message.data_base64,
                    MAX_INPUT_BYTES,
                )?;
            }
            ClientMessage::InterruptTerminal(message) => message.target.validate()?,
            ClientMessage::TerminalResize(message) => {
                message.target.validate()?;
                validate_dimensions(&message.dimensions)?;
            }
            ClientMessage::TerminalKey(message) => message.target.validate()?,
            ClientMessage::CreateProject(message) => {
                validate_opaque_id("project_id", &message.project_id)?;
                if let Some(cwd) = &message.cwd {
                    validate_path(cwd)?;
                }
            }
            ClientMessage::CreateSession(message) => {
                validate_opaque_id("project_id", &message.project_id)?;
                if let Some(cwd) = &message.cwd {
                    validate_path(cwd)?;
                }
                if let Some(prompt) = &message.initial_prompt {
                    validate_text("initial_prompt", prompt, 0, MAX_PROMPT_BYTES)?;
                }
            }
            ClientMessage::ResumeSession(message) => {
                validate_opaque_id("project_id", &message.project_id)?;
                validate_text(
                    "durable_session_id",
                    &message.durable_session_id,
                    1,
                    MAX_OPAQUE_ID_BYTES,
                )?;
                validate_path(&message.cwd)?;
            }
            ClientMessage::QuickInsertPreview(message) => {
                message.target.validate()?;
                validate_opaque_id("quick_insert_id", &message.item_id)?;
            }
            ClientMessage::QuickInsertSubmit(message) => {
                message.target.validate()?;
                validate_opaque_id("quick_insert_id", &message.item_id)?;
            }
            ClientMessage::UploadBegin(message) => {
                message.target.validate()?;
                validate_filename(&message.filename)?;
                validate_text("mime", &message.mime, 1, MAX_MIME_BYTES)?;
                if message.size == 0 || message.size > MAX_UPLOAD_BYTES {
                    return Err(ProtocolValidationError::UploadSize(message.size));
                }
                if message.sha256.len() != 64
                    || !message.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err(ProtocolValidationError::InvalidSha256);
                }
            }
            ClientMessage::Ping
            | ClientMessage::RequestSnapshot
            | ClientMessage::UploadCommit(_)
            | ClientMessage::UploadCancel(_)
            | ClientMessage::Disconnect => {}
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize, TS)]
pub struct ServerEnvelope {
    pub version: u16,
    pub request_id: Option<RequestId>,
    #[ts(type = "number")]
    pub sequence: Option<u64>,
    pub payload: ServerMessage,
}

/// Schema root used by the checked-in JSON Schema and TypeScript generator.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize, TS)]
pub struct CompanionProtocolSchema {
    pub client_envelope: ClientEnvelope,
    pub server_envelope: ServerEnvelope,
    pub pairing_claim_request: PairingClaimRequest,
    pub pairing_claim_receipt: PairingClaimReceipt,
    pub pairing_status_request: PairingStatusRequest,
    pub pairing_status: PairingStatus,
    pub auth_challenge_request: AuthChallengeRequest,
    pub auth_challenge: AuthChallenge,
    pub authenticate: Authenticate,
    pub pairing_invitation: PairingInvitation,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ProtocolValidationError {
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u16),
    #[error("{field} must contain between {min} and {max} UTF-8 bytes")]
    InvalidTextLength {
        field: &'static str,
        min: usize,
        max: usize,
    },
    #[error("{0} is not a valid opaque identifier")]
    InvalidOpaqueId(&'static str),
    #[error("terminal dimensions are outside the supported range")]
    InvalidDimensions,
    #[error("path is empty, too long, or contains a NUL byte")]
    InvalidPath,
    #[error("filename is unsafe")]
    InvalidFilename,
    #[error("upload size {0} is outside the supported range")]
    UploadSize(u64),
    #[error("sha256 must be 64 hexadecimal characters")]
    InvalidSha256,
    #[error("{0} must be non-empty base64 within the configured decoded size limit")]
    InvalidBase64Payload(&'static str),
    #[error("P-256 public key must be a base64-encoded 65-byte uncompressed SEC1 point")]
    InvalidP256PublicKey,
    #[error("P-256 signature must be base64-encoded IEEE P1363 r || s bytes")]
    InvalidP256Signature,
}

fn validate_text(
    field: &'static str,
    value: &str,
    min: usize,
    max: usize,
) -> Result<(), ProtocolValidationError> {
    if value.len() < min || value.len() > max {
        return Err(ProtocolValidationError::InvalidTextLength { field, min, max });
    }
    Ok(())
}

fn validate_opaque_id(field: &'static str, value: &str) -> Result<(), ProtocolValidationError> {
    if value.is_empty()
        || value.len() > MAX_OPAQUE_ID_BYTES
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(ProtocolValidationError::InvalidOpaqueId(field));
    }
    Ok(())
}

fn validate_base64_payload(
    field: &'static str,
    value: &str,
    max_decoded_bytes: usize,
) -> Result<(), ProtocolValidationError> {
    if value.is_empty() || value.len() > max_decoded_bytes.saturating_add(2) / 3 * 4 {
        return Err(ProtocolValidationError::InvalidBase64Payload(field));
    }

    let decoded = BASE64_STANDARD
        .decode(value)
        .map_err(|_| ProtocolValidationError::InvalidBase64Payload(field))?;
    if decoded.is_empty() || decoded.len() > max_decoded_bytes {
        return Err(ProtocolValidationError::InvalidBase64Payload(field));
    }

    Ok(())
}

fn validate_dimensions(dimensions: &TerminalDimensions) -> Result<(), ProtocolValidationError> {
    if dimensions.columns < 2
        || dimensions.rows < 1
        || dimensions.columns > 1000
        || dimensions.rows > 1000
    {
        return Err(ProtocolValidationError::InvalidDimensions);
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), ProtocolValidationError> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES || path.contains('\0') {
        return Err(ProtocolValidationError::InvalidPath);
    }
    Ok(())
}

fn validate_filename(filename: &str) -> Result<(), ProtocolValidationError> {
    if filename.is_empty()
        || filename.len() > MAX_FILENAME_BYTES
        || matches!(filename, "." | "..")
        || filename.contains(['/', '\\', '\0'])
        || filename.chars().any(char::is_control)
    {
        return Err(ProtocolValidationError::InvalidFilename);
    }
    Ok(())
}

pub const BINARY_FRAME_MAGIC: &[u8; 2] = b"CR";
pub const BINARY_FRAME_HEADER_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum BinaryFrameKind {
    TerminalOutput = 1,
    UploadChunk = 2,
}

impl TryFrom<u8> for BinaryFrameKind {
    type Error = BinaryFrameError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::TerminalOutput),
            2 => Ok(Self::UploadChunk),
            _ => Err(BinaryFrameError::UnknownKind(value)),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct UploadChunkFrame<'a> {
    pub upload_id: UploadId,
    pub chunk_index: u64,
    pub payload: &'a [u8],
}

#[derive(Debug, Eq, PartialEq)]
pub struct TerminalOutputFrame<'a> {
    pub stream_id: TerminalStreamId,
    pub terminal_sequence: u64,
    pub payload: &'a [u8],
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BinaryFrameError {
    #[error("binary frame is shorter than the fixed header")]
    TooShort,
    #[error("binary frame magic is invalid")]
    InvalidMagic,
    #[error("binary frame protocol version {0} is unsupported")]
    UnsupportedVersion(u16),
    #[error("binary frame kind {0} is unknown")]
    UnknownKind(u8),
    #[error("binary frame reserved flags are non-zero")]
    InvalidFlags,
    #[error("expected binary frame kind {expected}, received {actual}")]
    UnexpectedKind { expected: u8, actual: u8 },
    #[error("binary upload chunk exceeds the configured limit")]
    ChunkTooLarge,
    #[error("binary terminal frame exceeds the configured limit")]
    TerminalFrameTooLarge,
}

pub fn encode_upload_chunk(
    upload_id: UploadId,
    chunk_index: u64,
    payload: &[u8],
) -> Result<Vec<u8>, BinaryFrameError> {
    if payload.len() > MAX_UPLOAD_CHUNK_BYTES {
        return Err(BinaryFrameError::ChunkTooLarge);
    }

    Ok(encode_binary_frame(
        BinaryFrameKind::UploadChunk,
        upload_id.0,
        chunk_index,
        payload,
    ))
}

pub fn decode_upload_chunk(frame: &[u8]) -> Result<UploadChunkFrame<'_>, BinaryFrameError> {
    let (kind, id, chunk_index, payload) = decode_binary_frame(frame)?;
    if kind != BinaryFrameKind::UploadChunk {
        return Err(BinaryFrameError::UnexpectedKind {
            expected: BinaryFrameKind::UploadChunk as u8,
            actual: kind as u8,
        });
    }
    if payload.len() > MAX_UPLOAD_CHUNK_BYTES {
        return Err(BinaryFrameError::ChunkTooLarge);
    }

    Ok(UploadChunkFrame {
        upload_id: UploadId(id),
        chunk_index,
        payload,
    })
}

pub fn encode_terminal_output(
    stream_id: TerminalStreamId,
    terminal_sequence: u64,
    payload: &[u8],
) -> Result<Vec<u8>, BinaryFrameError> {
    if payload.len() > MAX_TERMINAL_FRAME_BYTES {
        return Err(BinaryFrameError::TerminalFrameTooLarge);
    }

    Ok(encode_binary_frame(
        BinaryFrameKind::TerminalOutput,
        stream_id.0,
        terminal_sequence,
        payload,
    ))
}

pub fn decode_terminal_output(frame: &[u8]) -> Result<TerminalOutputFrame<'_>, BinaryFrameError> {
    let (kind, id, terminal_sequence, payload) = decode_binary_frame(frame)?;
    if kind != BinaryFrameKind::TerminalOutput {
        return Err(BinaryFrameError::UnexpectedKind {
            expected: BinaryFrameKind::TerminalOutput as u8,
            actual: kind as u8,
        });
    }
    if payload.len() > MAX_TERMINAL_FRAME_BYTES {
        return Err(BinaryFrameError::TerminalFrameTooLarge);
    }

    Ok(TerminalOutputFrame {
        stream_id: TerminalStreamId(id),
        terminal_sequence,
        payload,
    })
}

fn encode_binary_frame(kind: BinaryFrameKind, id: Uuid, sequence: u64, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(BINARY_FRAME_HEADER_BYTES + payload.len());
    frame.extend_from_slice(BINARY_FRAME_MAGIC);
    frame.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    frame.push(kind as u8);
    frame.extend_from_slice(&[0_u8; 3]);
    frame.extend_from_slice(id.as_bytes());
    frame.extend_from_slice(&sequence.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn decode_binary_frame(
    frame: &[u8],
) -> Result<(BinaryFrameKind, Uuid, u64, &[u8]), BinaryFrameError> {
    if frame.len() < BINARY_FRAME_HEADER_BYTES {
        return Err(BinaryFrameError::TooShort);
    }
    if &frame[..2] != BINARY_FRAME_MAGIC {
        return Err(BinaryFrameError::InvalidMagic);
    }

    let version = u16::from_be_bytes([frame[2], frame[3]]);
    if version != PROTOCOL_VERSION {
        return Err(BinaryFrameError::UnsupportedVersion(version));
    }

    let kind = BinaryFrameKind::try_from(frame[4])?;
    if frame[5..8] != [0_u8; 3] {
        return Err(BinaryFrameError::InvalidFlags);
    }
    let id = Uuid::from_bytes(frame[8..24].try_into().expect("fixed upload UUID slice"));
    let sequence = u64::from_be_bytes(
        frame[24..32]
            .try_into()
            .expect("fixed binary frame sequence slice"),
    );
    let payload = &frame[BINARY_FRAME_HEADER_BYTES..];
    Ok((kind, id, sequence, payload))
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

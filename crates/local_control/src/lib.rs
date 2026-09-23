//! Shared protocol, discovery, authentication, and client types for local Warp control.
//!
//! The `local_control` crate is intentionally UI-agnostic so the Warp app and
//! `warpctrl` CLI can share the same wire envelopes, action catalog, discovery
//! records, selectors, and credential validation rules.
pub mod auth;
pub mod catalog;
pub mod client;
pub mod discovery;
pub mod protocol;
pub mod selection;
pub mod selectors;

pub use auth::{AuthToken, CredentialGrant, CredentialRequest, ScopedCredential};
pub use catalog::{ActionImplementationStatus, ActionKind, ActionMetadata, TargetScope};
pub use discovery::{
    ControlEndpoint, CredentialBrokerReference, InstanceId, InstanceRecord, RegisteredInstance,
    discovery_dir,
};
pub use protocol::{
    Action, ControlError, ControlResponse, ErrorCode, ErrorResponseEnvelope, PROTOCOL_VERSION,
    RequestEnvelope, ResponseEnvelope,
};
pub use selectors::{PaneSelector, SessionSelector, TabSelector, TargetSelector, WindowSelector};

/// Durable terminal identity exported to processes launched inside a Warp session.
///
/// Local-control clients use this to preserve the originating project workspace
/// when a physical window contains more than one project.
pub const TERMINAL_SESSION_UUID_ENV: &str = "WARP_TERMINAL_SESSION_UUID";

/// Process identity exported by a Clinch-bound terminal for its owning app.
pub const CLINCH_CONTROL_PID_ENV: &str = "CLINCH_CONTROL_PID";

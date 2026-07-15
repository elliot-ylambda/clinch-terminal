//! Local two-way iMessage bridge for durable CLI-agent sessions.
//!
//! The domain and persistence layers deliberately contain no Messages or PTY
//! side effects. The coordinator is the only owner of the native helper and of
//! routing replies into a live terminal view.

mod bridge;
mod coordinator;
mod domain;
mod protocol;
mod store;

pub(crate) use coordinator::{
    IMessageConnectionStatus, IMessageCoordinator, IMessageCoordinatorEvent,
};

#[cfg(test)]
mod tests;

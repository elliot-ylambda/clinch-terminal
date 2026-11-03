//! Shared retry primitives for HTTP-backed operations in the agent SDK.
//!
//! The canonical implementation of `with_bounded_retry` and `is_transient_http_error`
//! lives in [`crate::server::retry_strategies`] (available on all targets, including WASM).
//! This module re-exports those symbols so existing agent-SDK call sites keep compiling
//! without a path change.

pub(crate) use crate::server::retry_strategies::with_bounded_retry;

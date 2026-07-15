use std::collections::HashSet;
#[cfg(test)]
use std::collections::{HashMap, VecDeque};
use std::fmt;

use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::agent_resume::AgentResumeProvider;

pub(crate) const MAX_IMESSAGE_CHARS: usize = 3_000;
const MAX_ROUTE_LABEL_CHARS: usize = 80;
const ROUTE_CODE_LEN: usize = 4;
const ROUTE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const ROUTE_QUARANTINE_SECONDS: i64 = 30 * 24 * 60 * 60;
const PENDING_SELECTION_TTL_SECONDS: i64 = 10 * 60;
const QUEUED_REPLY_TTL_SECONDS: i64 = 24 * 60 * 60;
const OUTBOUND_INTENT_TTL_SECONDS: i64 = 24 * 60 * 60;
const PROCESSED_GUID_RETENTION_SECONDS: i64 = 30 * 24 * 60 * 60;
const MAX_PROCESSED_GUIDS: usize = 20_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MobileProvider {
    Claude,
    Codex,
}

impl MobileProvider {
    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }
}

impl From<AgentResumeProvider> for MobileProvider {
    fn from(value: AgentResumeProvider) -> Self {
        match value {
            AgentResumeProvider::Claude => Self::Claude,
            AgentResumeProvider::Codex => Self::Codex,
        }
    }
}

impl From<MobileProvider> for AgentResumeProvider {
    fn from(value: MobileProvider) -> Self {
        match value {
            MobileProvider::Claude => Self::Claude,
            MobileProvider::Codex => Self::Codex,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct MobileSessionKey {
    pub(crate) provider: MobileProvider,
    pub(crate) session_id: String,
}

impl MobileSessionKey {
    pub(crate) fn new(provider: MobileProvider, session_id: impl Into<String>) -> Option<Self> {
        let session_id = session_id.into();
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return None;
        }
        Some(Self {
            provider,
            session_id: session_id.to_owned(),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct MobileRouteId(String);

impl MobileRouteId {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_uppercase();
        if normalized.len() != ROUTE_CODE_LEN
            || !normalized
                .bytes()
                .all(|character| ROUTE_ALPHABET.contains(&character))
            || !normalized
                .bytes()
                .any(|character| character.is_ascii_digit())
            || !normalized
                .bytes()
                .any(|character| character.is_ascii_alphabetic())
        {
            return None;
        }
        Some(Self(normalized))
    }
}

impl fmt::Display for MobileRouteId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct MobileSessionRoute {
    pub(crate) id: MobileRouteId,
    pub(crate) key: MobileSessionKey,
    pub(crate) label: String,
    pub(crate) active: bool,
    /// Reads the original persisted `opted_out` field. New writes use
    /// `notification_override`, but retaining this field makes an existing
    /// explicit opt-out continue to mean off after upgrading.
    #[serde(default, rename = "opted_out", skip_serializing_if = "is_false")]
    pub(crate) legacy_opted_out: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) notification_override: Option<bool>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

impl MobileSessionRoute {
    pub(crate) fn notifications_enabled(&self, enabled_by_default: bool) -> bool {
        self.notification_override
            .unwrap_or(!self.legacy_opted_out && enabled_by_default)
    }

    pub(crate) fn is_eligible(
        &self,
        globally_enabled: bool,
        enabled_by_default: bool,
    ) -> bool {
        globally_enabled && self.active && self.notifications_enabled(enabled_by_default)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct OutboundMessageRoute {
    pub(crate) guid: String,
    pub(crate) route_id: Option<MobileRouteId>,
    pub(crate) recorded_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PendingOutboundIntent {
    pub(crate) id: String,
    pub(crate) text_sha256: String,
    pub(crate) route_id: Option<MobileRouteId>,
    pub(crate) after_row_id: i64,
    pub(crate) created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PendingCalibration {
    pub(crate) expected_reply: String,
    pub(crate) sent_guid: String,
    pub(crate) created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ProcessedMessage {
    pub(crate) guid: String,
    pub(crate) processed_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct RetiredRoute {
    pub(crate) id: MobileRouteId,
    pub(crate) retired_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct QueuedMobileReply {
    pub(crate) id: String,
    pub(crate) source_guid: String,
    pub(crate) route_id: MobileRouteId,
    pub(crate) text: String,
    pub(crate) queued_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PendingRouteSelection {
    pub(crate) id: String,
    pub(crate) source_guid: String,
    pub(crate) text: String,
    pub(crate) candidate_route_ids: Vec<MobileRouteId>,
    pub(crate) created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct IncomingMessage {
    pub(crate) guid: String,
    pub(crate) row_id: i64,
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) service: String,
    #[serde(default)]
    pub(crate) parent_guid: Option<String>,
    #[serde(default)]
    pub(crate) associated_guid: Option<String>,
    #[serde(default)]
    pub(crate) is_reaction: bool,
    #[serde(default)]
    pub(crate) is_edited: bool,
    #[serde(default)]
    pub(crate) has_attachments: bool,
    #[serde(default)]
    pub(crate) is_from_me: bool,
}

impl IncomingMessage {
    pub(crate) fn is_supported_text(&self) -> bool {
        !self.guid.trim().is_empty()
            && self.service.eq_ignore_ascii_case("imessage")
            && !self.is_reaction
            && !self.is_edited
            && !self.has_attachments
            && !self.text.trim().is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RouteDecision {
    Ignore,
    Duplicate,
    Deliver {
        route_id: MobileRouteId,
        text: String,
    },
    Ambiguous {
        pending_id: String,
        candidate_route_ids: Vec<MobileRouteId>,
    },
    NoPendingSelection(MobileRouteId),
    UnknownRoute(MobileRouteId),
    NoEligibleRoute,
}

#[derive(Debug, Default)]
pub(crate) struct ExpiredItems {
    pub(crate) pending_selections: Vec<PendingRouteSelection>,
    pub(crate) queued_replies: Vec<QueuedMobileReply>,
    pub(crate) state_changed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct RouteState {
    pub(crate) version: u32,
    pub(crate) globally_enabled: bool,
    #[serde(default = "default_true")]
    pub(crate) notifications_enabled_by_default: bool,
    pub(crate) last_row_id: i64,
    pub(crate) routes: Vec<MobileSessionRoute>,
    pub(crate) retired_routes: Vec<RetiredRoute>,
    pub(crate) outbound_messages: Vec<OutboundMessageRoute>,
    #[serde(default)]
    pub(crate) pending_outbound_intents: Vec<PendingOutboundIntent>,
    pub(crate) processed_messages: Vec<ProcessedMessage>,
    pub(crate) pending_selections: Vec<PendingRouteSelection>,
    pub(crate) queued_replies: Vec<QueuedMobileReply>,
    pub(crate) pending_calibration: Option<PendingCalibration>,
}

impl Default for RouteState {
    fn default() -> Self {
        Self {
            version: 1,
            globally_enabled: false,
            notifications_enabled_by_default: true,
            last_row_id: 0,
            routes: Vec::new(),
            retired_routes: Vec::new(),
            outbound_messages: Vec::new(),
            pending_outbound_intents: Vec::new(),
            processed_messages: Vec::new(),
            pending_selections: Vec::new(),
            queued_replies: Vec::new(),
            pending_calibration: None,
        }
    }
}

impl RouteState {
    pub(crate) fn migrate_legacy_notification_overrides(&mut self) -> bool {
        let mut changed = false;
        for route in &mut self.routes {
            if route.notification_override.is_none() && route.legacy_opted_out {
                route.notification_override = Some(false);
                changed = true;
            }
            if route.legacy_opted_out {
                route.legacy_opted_out = false;
                changed = true;
            }
        }
        changed
    }

    pub(crate) fn register_session(
        &mut self,
        key: MobileSessionKey,
        label: impl Into<String>,
        now: i64,
    ) -> MobileRouteId {
        let label = sanitize_label(&label.into());
        if let Some(index) = self.routes.iter().position(|route| route.key == key) {
            let existing = &mut self.routes[index];
            existing.active = true;
            existing.label = label;
            existing.updated_at = now;
            let id = existing.id.clone();
            self.retired_routes.retain(|retired| retired.id != id);
            return id;
        }

        let unavailable = self.unavailable_route_ids(now);
        let id = generate_route_id(&unavailable);
        self.routes.push(MobileSessionRoute {
            id: id.clone(),
            key,
            label,
            active: true,
            legacy_opted_out: false,
            notification_override: None,
            created_at: now,
            updated_at: now,
        });
        id
    }

    pub(crate) fn deactivate_all_sessions(&mut self, now: i64) {
        let mut newly_retired = Vec::new();
        for route in &mut self.routes {
            if route.active {
                route.active = false;
                route.updated_at = now;
                newly_retired.push(route.id.clone());
            }
        }
        for id in newly_retired {
            if !self.retired_routes.iter().any(|retired| retired.id == id) {
                self.retired_routes.push(RetiredRoute {
                    id,
                    retired_at: now,
                });
            }
        }
    }

    pub(crate) fn retire_session(
        &mut self,
        key: &MobileSessionKey,
        now: i64,
    ) -> Vec<QueuedMobileReply> {
        let Some(route) = self.routes.iter_mut().find(|route| &route.key == key) else {
            return Vec::new();
        };
        route.active = false;
        route.updated_at = now;
        let id = route.id.clone();
        if !self.retired_routes.iter().any(|retired| retired.id == id) {
            self.retired_routes.push(RetiredRoute {
                id: id.clone(),
                retired_at: now,
            });
        }
        self.take_queued_for_route(&id)
    }

    pub(crate) fn set_notifications_enabled(
        &mut self,
        key: &MobileSessionKey,
        enabled: bool,
        now: i64,
    ) -> Vec<QueuedMobileReply> {
        let Some(route) = self.routes.iter_mut().find(|route| &route.key == key) else {
            return Vec::new();
        };
        route.legacy_opted_out = false;
        route.notification_override = Some(enabled);
        route.updated_at = now;
        let id = route.id.clone();
        if !enabled {
            self.take_queued_for_route(&id)
        } else {
            Vec::new()
        }
    }

    pub(crate) fn route_for_key(&self, key: &MobileSessionKey) -> Option<&MobileSessionRoute> {
        self.routes.iter().find(|route| &route.key == key)
    }

    pub(crate) fn route_by_id(&self, id: &MobileRouteId) -> Option<&MobileSessionRoute> {
        self.routes.iter().find(|route| &route.id == id)
    }

    pub(crate) fn eligible_routes(&self) -> Vec<&MobileSessionRoute> {
        let mut routes = self
            .routes
            .iter()
            .filter(|route| {
                route.is_eligible(
                    self.globally_enabled,
                    self.notifications_enabled_by_default,
                )
            })
            .collect::<Vec<_>>();
        routes.sort_by(|left, right| left.id.cmp(&right.id));
        routes
    }

    pub(crate) fn record_outbound_guid(
        &mut self,
        guid: impl Into<String>,
        route_id: MobileRouteId,
        now: i64,
    ) {
        let guid = guid.into();
        if guid.trim().is_empty() {
            return;
        }
        if let Some(existing) = self
            .outbound_messages
            .iter_mut()
            .find(|message| message.guid == guid)
        {
            existing.route_id = Some(route_id);
            existing.recorded_at = now;
            return;
        }
        self.outbound_messages.push(OutboundMessageRoute {
            guid,
            route_id: Some(route_id),
            recorded_at: now,
        });
    }

    pub(crate) fn record_system_outbound_guid(&mut self, guid: impl Into<String>, now: i64) {
        let guid = guid.into();
        if guid.trim().is_empty() {
            return;
        }
        if let Some(existing) = self
            .outbound_messages
            .iter_mut()
            .find(|message| message.guid == guid)
        {
            existing.route_id = None;
            existing.recorded_at = now;
            return;
        }
        self.outbound_messages.push(OutboundMessageRoute {
            guid,
            route_id: None,
            recorded_at: now,
        });
    }

    pub(crate) fn record_outbound_intent(
        &mut self,
        text: &str,
        route_id: Option<MobileRouteId>,
        now: i64,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        self.pending_outbound_intents.push(PendingOutboundIntent {
            id: id.clone(),
            text_sha256: text_fingerprint(text),
            route_id,
            after_row_id: self.last_row_id,
            created_at: now,
        });
        id
    }

    pub(crate) fn resolve_outbound_intent(&mut self, id: &str) {
        self.pending_outbound_intents
            .retain(|intent| intent.id != id);
    }

    pub(crate) fn mark_processed(&mut self, guid: impl Into<String>, now: i64) {
        let guid = guid.into();
        if guid.trim().is_empty()
            || self
                .processed_messages
                .iter()
                .any(|message| message.guid == guid)
        {
            return;
        }
        self.processed_messages.push(ProcessedMessage {
            guid,
            processed_at: now,
        });
        if self.processed_messages.len() > MAX_PROCESSED_GUIDS {
            let remove = self.processed_messages.len() - MAX_PROCESSED_GUIDS;
            self.processed_messages.drain(..remove);
        }
    }

    pub(crate) fn route_incoming(&mut self, incoming: &IncomingMessage, now: i64) -> RouteDecision {
        self.prune(now);
        self.last_row_id = self.last_row_id.max(incoming.row_id);

        if incoming.guid.trim().is_empty()
            || self
                .processed_messages
                .iter()
                .any(|message| message.guid == incoming.guid)
        {
            return RouteDecision::Duplicate;
        }
        if !incoming.is_supported_text()
            || self
                .outbound_messages
                .iter()
                .any(|message| message.guid == incoming.guid)
        {
            return RouteDecision::Ignore;
        }

        // `is_from_me` is not a general inbound filter in a synchronized
        // self-chat. It is used only with a persisted pre-send fingerprint to
        // recover the exact GUID when the app exited after Messages accepted
        // the send but before the response was stored.
        if incoming.is_from_me {
            let fingerprint = text_fingerprint(&incoming.text);
            if let Some(index) = self.pending_outbound_intents.iter().position(|intent| {
                incoming.row_id > intent.after_row_id && intent.text_sha256 == fingerprint
            }) {
                let intent = self.pending_outbound_intents.remove(index);
                if let Some(route_id) = intent.route_id {
                    self.record_outbound_guid(incoming.guid.clone(), route_id, now);
                } else {
                    self.record_system_outbound_guid(incoming.guid.clone(), now);
                }
                return RouteDecision::Ignore;
            }
        }

        for referenced_guid in [
            incoming.parent_guid.as_deref(),
            incoming.associated_guid.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(outbound) = self
                .outbound_messages
                .iter()
                .find(|message| message.guid == referenced_guid)
            {
                let Some(route_id) = outbound.route_id.as_ref() else {
                    continue;
                };
                if self.route_is_eligible(route_id) {
                    return RouteDecision::Deliver {
                        route_id: route_id.clone(),
                        text: incoming.text.clone(),
                    };
                }
                return RouteDecision::NoEligibleRoute;
            }
        }

        if let Some((pending_index, route_id)) =
            self.pending_selection_for_code(&incoming.text, now)
        {
            let pending = self.pending_selections.remove(pending_index);
            return RouteDecision::Deliver {
                route_id,
                text: pending.text,
            };
        }

        if let Some(route_id) = parse_code_only(&incoming.text) {
            if self.route_is_eligible(&route_id) {
                return RouteDecision::NoPendingSelection(route_id);
            }
        }

        if let Some((route_id, stripped_text)) = parse_route_prefix(&incoming.text) {
            if !self.route_is_eligible(&route_id) {
                return RouteDecision::UnknownRoute(route_id);
            }
            if stripped_text.trim().is_empty() {
                return RouteDecision::Ignore;
            }
            return RouteDecision::Deliver {
                route_id,
                text: stripped_text,
            };
        }

        let eligible = self.eligible_routes();
        if eligible.len() == 1 {
            return RouteDecision::Deliver {
                route_id: eligible[0].id.clone(),
                text: incoming.text.clone(),
            };
        }
        if eligible.is_empty() {
            return RouteDecision::NoEligibleRoute;
        }

        let candidate_route_ids = eligible
            .into_iter()
            .map(|route| route.id.clone())
            .collect::<Vec<_>>();
        let pending_id = Uuid::new_v4().to_string();
        self.pending_selections.push(PendingRouteSelection {
            id: pending_id.clone(),
            source_guid: incoming.guid.clone(),
            text: incoming.text.clone(),
            candidate_route_ids: candidate_route_ids.clone(),
            created_at: now,
        });
        RouteDecision::Ambiguous {
            pending_id,
            candidate_route_ids,
        }
    }

    pub(crate) fn enqueue_reply(
        &mut self,
        source_guid: impl Into<String>,
        route_id: MobileRouteId,
        text: impl Into<String>,
        now: i64,
    ) -> Option<String> {
        if !self.route_is_eligible(&route_id) {
            return None;
        }
        let id = Uuid::new_v4().to_string();
        self.queued_replies.push(QueuedMobileReply {
            id: id.clone(),
            source_guid: source_guid.into(),
            route_id,
            text: text.into(),
            queued_at: now,
        });
        Some(id)
    }

    pub(crate) fn has_queued_for_route(&self, route_id: &MobileRouteId) -> bool {
        self.queued_replies
            .iter()
            .any(|reply| &reply.route_id == route_id)
    }

    pub(crate) fn pop_next_queued(
        &mut self,
        route_id: &MobileRouteId,
        now: i64,
    ) -> Option<QueuedMobileReply> {
        self.prune(now);
        let index = self
            .queued_replies
            .iter()
            .enumerate()
            .filter(|(_, reply)| &reply.route_id == route_id)
            .min_by_key(|(_, reply)| reply.queued_at)
            .map(|(index, _)| index)?;
        Some(self.queued_replies.remove(index))
    }

    pub(crate) fn take_queued_for_route(
        &mut self,
        route_id: &MobileRouteId,
    ) -> Vec<QueuedMobileReply> {
        let mut retained = Vec::with_capacity(self.queued_replies.len());
        let mut removed = Vec::new();
        for reply in self.queued_replies.drain(..) {
            if &reply.route_id == route_id {
                removed.push(reply);
            } else {
                retained.push(reply);
            }
        }
        self.queued_replies = retained;
        removed.sort_by_key(|reply| reply.queued_at);
        removed
    }

    pub(crate) fn prune(&mut self, now: i64) {
        let _ = self.take_expired(now);
    }

    pub(crate) fn take_expired(&mut self, now: i64) -> ExpiredItems {
        let mut state_changed = false;
        let expired_route_ids = self
            .retired_routes
            .iter()
            .filter(|retired| now.saturating_sub(retired.retired_at) >= ROUTE_QUARANTINE_SECONDS)
            .map(|retired| retired.id.clone())
            .collect::<HashSet<_>>();
        if !expired_route_ids.is_empty() {
            let before = self.routes.len();
            self.routes
                .retain(|route| route.active || !expired_route_ids.contains(&route.id));
            state_changed |= before != self.routes.len();
        }

        let before = self.retired_routes.len();
        self.retired_routes
            .retain(|retired| now.saturating_sub(retired.retired_at) < ROUTE_QUARANTINE_SECONDS);
        state_changed |= before != self.retired_routes.len();

        let mut pending_selections = Vec::new();
        self.pending_selections.retain(|pending| {
            let retain = now.saturating_sub(pending.created_at) < PENDING_SELECTION_TTL_SECONDS;
            if !retain {
                pending_selections.push(pending.clone());
            }
            retain
        });
        state_changed |= !pending_selections.is_empty();

        let mut queued_replies = Vec::new();
        self.queued_replies.retain(|reply| {
            let retain = now.saturating_sub(reply.queued_at) < QUEUED_REPLY_TTL_SECONDS;
            if !retain {
                queued_replies.push(reply.clone());
            }
            retain
        });
        state_changed |= !queued_replies.is_empty();

        let before = self.processed_messages.len();
        self.processed_messages.retain(|message| {
            now.saturating_sub(message.processed_at) < PROCESSED_GUID_RETENTION_SECONDS
        });
        state_changed |= before != self.processed_messages.len();

        let before = self.outbound_messages.len();
        self.outbound_messages.retain(|message| {
            now.saturating_sub(message.recorded_at) < PROCESSED_GUID_RETENTION_SECONDS
        });
        state_changed |= before != self.outbound_messages.len();

        let before = self.pending_outbound_intents.len();
        self.pending_outbound_intents
            .retain(|intent| now.saturating_sub(intent.created_at) < OUTBOUND_INTENT_TTL_SECONDS);
        state_changed |= before != self.pending_outbound_intents.len();

        ExpiredItems {
            pending_selections,
            queued_replies,
            state_changed,
        }
    }

    pub(crate) fn next_expiration_at(&self) -> Option<i64> {
        self.pending_selections
            .iter()
            .map(|pending| {
                pending
                    .created_at
                    .saturating_add(PENDING_SELECTION_TTL_SECONDS)
            })
            .chain(
                self.queued_replies
                    .iter()
                    .map(|reply| reply.queued_at.saturating_add(QUEUED_REPLY_TTL_SECONDS)),
            )
            .chain(self.pending_outbound_intents.iter().map(|intent| {
                intent
                    .created_at
                    .saturating_add(OUTBOUND_INTENT_TTL_SECONDS)
            }))
            .min()
    }

    pub(crate) fn reset_conversation_state(&mut self) {
        self.last_row_id = 0;
        self.outbound_messages.clear();
        self.pending_outbound_intents.clear();
        self.processed_messages.clear();
        self.pending_selections.clear();
        self.queued_replies.clear();
        self.pending_calibration = None;
    }

    fn route_is_eligible(&self, id: &MobileRouteId) -> bool {
        self.route_by_id(id).is_some_and(|route| {
            route.is_eligible(
                self.globally_enabled,
                self.notifications_enabled_by_default,
            )
        })
    }

    fn unavailable_route_ids(&self, now: i64) -> HashSet<MobileRouteId> {
        self.routes
            .iter()
            .map(|route| route.id.clone())
            .chain(
                self.retired_routes
                    .iter()
                    .filter(|retired| {
                        now.saturating_sub(retired.retired_at) < ROUTE_QUARANTINE_SECONDS
                    })
                    .map(|retired| retired.id.clone()),
            )
            .collect()
    }

    fn pending_selection_for_code(&self, text: &str, now: i64) -> Option<(usize, MobileRouteId)> {
        let route_id = parse_code_only(text)?;
        self.pending_selections
            .iter()
            .enumerate()
            .rev()
            .find(|(_, pending)| {
                now.saturating_sub(pending.created_at) < PENDING_SELECTION_TTL_SECONDS
                    && pending.candidate_route_ids.contains(&route_id)
                    && self.route_is_eligible(&route_id)
            })
            .map(|(index, _)| (index, route_id))
    }
}

fn text_fingerprint(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

fn default_true() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn generate_route_id(unavailable: &HashSet<MobileRouteId>) -> MobileRouteId {
    let mut rng = rand::thread_rng();
    loop {
        let value = (0..ROUTE_CODE_LEN)
            .map(|_| {
                *ROUTE_ALPHABET
                    .choose(&mut rng)
                    .expect("route alphabet is non-empty") as char
            })
            .collect::<String>();
        let candidate = MobileRouteId(value);
        if MobileRouteId::parse(&candidate.0).is_some() && !unavailable.contains(&candidate) {
            return candidate;
        }
    }
}

fn parse_code_only(text: &str) -> Option<MobileRouteId> {
    let trimmed = text.trim();
    let unwrapped = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(trimmed);
    MobileRouteId::parse(unwrapped)
}

fn parse_route_prefix(text: &str) -> Option<(MobileRouteId, String)> {
    let trimmed_start = text.trim_start();
    if let Some(after_open) = trimmed_start.strip_prefix('[') {
        let close = after_open.find(']')?;
        let route_id = MobileRouteId::parse(&after_open[..close])?;
        let remainder = after_open[close + 1..]
            .trim_start_matches(|character: char| character == ':' || character.is_whitespace())
            .to_owned();
        return Some((route_id, remainder));
    }

    let prefix_end = trimmed_start
        .char_indices()
        .find(|(_, character)| character.is_whitespace() || *character == ':')
        .map(|(index, _)| index)
        .unwrap_or(trimmed_start.len());
    let route_id = MobileRouteId::parse(&trimmed_start[..prefix_end])?;
    let remainder = trimmed_start[prefix_end..]
        .trim_start_matches(|character: char| character == ':' || character.is_whitespace())
        .to_owned();
    Some((route_id, remainder))
}

pub(crate) fn format_completion_messages(
    route_id: &MobileRouteId,
    provider: MobileProvider,
    label: &str,
    response: Option<&str>,
) -> Vec<String> {
    let label = sanitize_label(label);
    let label = if label.is_empty() {
        "Agent session".to_owned()
    } else {
        label
    };
    let body = response
        .filter(|response| !response.trim().is_empty())
        .unwrap_or("The agent finished. Open Clinch to view the result.");

    // Reserve enough space for the route, provider, truncated label, and even
    // four-digit part counters. Every body byte is retained across chunks.
    let header_reserve = 160;
    let chunks = split_text_preserving_content(body, MAX_IMESSAGE_CHARS - header_reserve);
    let part_count = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let part = if part_count == 1 {
                String::new()
            } else {
                format!(" ({}/{part_count})", index + 1)
            };
            let header = format!(
                "[{route_id}] {} · {label} · Done{part}",
                provider.display_name()
            );
            let message = format!("{header}\n\n{chunk}");
            debug_assert!(message.chars().count() <= MAX_IMESSAGE_CHARS);
            message
        })
        .collect()
}

fn split_text_preserving_content(text: &str, max_chars: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut parts = Vec::new();
    let mut remaining = text;
    while remaining.chars().count() > max_chars {
        let split_byte = remaining
            .char_indices()
            .nth(max_chars)
            .map(|(index, _)| index)
            .unwrap_or(remaining.len());
        let (part, rest) = remaining.split_at(split_byte);
        parts.push(part.to_owned());
        remaining = rest;
    }
    parts.push(remaining.to_owned());
    parts
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn sanitize_label(value: &str) -> String {
    let printable = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let normalized = printable.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&normalized, MAX_ROUTE_LABEL_CHARS)
}

pub(crate) fn sanitize_incoming_text(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|character| matches!(character, '\n' | '\t') || !character.is_control())
        .collect()
}

#[cfg(test)]
pub(super) fn queued_by_route(
    state: &RouteState,
) -> HashMap<MobileRouteId, VecDeque<&QueuedMobileReply>> {
    let mut result: HashMap<MobileRouteId, VecDeque<&QueuedMobileReply>> = HashMap::new();
    for reply in &state.queued_replies {
        result
            .entry(reply.route_id.clone())
            .or_default()
            .push_back(reply);
    }
    result
}

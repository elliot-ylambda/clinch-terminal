use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use settings::Setting as _;
use uuid::Uuid;
use warpui::r#async::Timer;
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity, ViewHandle};

use super::bridge::{bridge_executable_path, IMessageBridge, IMessageBridgeEvent};
use super::domain::{
    format_completion_messages, sanitize_incoming_text, MobileProvider, MobileRouteId,
    MobileSessionKey, PendingCalibration, RouteDecision, RouteState,
};
use super::protocol::{
    BridgeCommand, BridgeEvent, BridgePermission, BridgeResponse, BridgeResult,
    BRIDGE_PROTOCOL_VERSION,
};
use super::store::RouteStateStore;
use crate::settings::{ClinchSettings, IMessageConfiguration};
use crate::terminal::cli_agent_sessions::{
    CLIAgentSessionKey, CLIAgentSessionStatus, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};
use crate::terminal::view::TerminalView;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IMessagePermission {
    Automation,
    FullDiskAccess,
    MessagesSignIn,
}

impl From<BridgePermission> for IMessagePermission {
    fn from(value: BridgePermission) -> Self {
        match value {
            BridgePermission::Automation => Self::Automation,
            BridgePermission::FullDiskAccess => Self::FullDiskAccess,
            BridgePermission::MessagesSignIn => Self::MessagesSignIn,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum IMessageConnectionStatus {
    Disabled,
    SetupRequired,
    Connecting,
    AwaitingCalibrationReply,
    Connected,
    Paused(IMessagePermission),
    Error,
}

impl IMessageConnectionStatus {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Disabled => "iMessage off",
            Self::SetupRequired => "Set up iMessage",
            Self::Connecting => "Connecting iMessage…",
            Self::AwaitingCalibrationReply => "Reply to the setup message",
            Self::Connected => "iMessage connected",
            Self::Paused(IMessagePermission::Automation) => "Allow Messages automation",
            Self::Paused(IMessagePermission::FullDiskAccess) => "Allow Full Disk Access",
            Self::Paused(IMessagePermission::MessagesSignIn) => "Sign in to Messages",
            Self::Error => "iMessage unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum IMessageTestStatus {
    #[default]
    Idle,
    Sending,
    Sent,
    Failed,
}

#[derive(Clone, Debug)]
pub(crate) enum IMessageCoordinatorEvent {
    Changed,
    SessionChanged { terminal_view_id: EntityId },
}

#[derive(Clone, Debug)]
struct OutboundJob {
    route_id: Option<MobileRouteId>,
    parts: VecDeque<String>,
    drain_after_send: bool,
    kind: OutboundJobKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutboundJobKind {
    Message,
    ConnectionTest,
}

pub(crate) struct IMessageCoordinator {
    status: IMessageConnectionStatus,
    configuration: IMessageConfiguration,
    state: RouteState,
    store: RouteStateStore,
    bridge: Option<Arc<IMessageBridge>>,
    view_sessions: HashMap<EntityId, MobileSessionKey>,
    session_views: HashMap<MobileSessionKey, EntityId>,
    outbound_jobs: VecDeque<OutboundJob>,
    mobile_submissions_in_flight: HashSet<MobileSessionKey>,
    send_in_flight: bool,
    test_status: IMessageTestStatus,
    current_send_intent_id: Option<String>,
    bridge_generation: u64,
    bridge_restart_attempts: usize,
    bridge_restart_scheduled: bool,
    expiration_generation: u64,
}

impl Entity for IMessageCoordinator {
    type Event = IMessageCoordinatorEvent;
}

impl SingletonEntity for IMessageCoordinator {}

impl IMessageCoordinator {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        let store = RouteStateStore::default();
        let mut state = store.load().unwrap_or_else(|error| {
            log::warn!("Could not restore local iMessage routing state: {error:#}");
            RouteState::default()
        });
        // Runtime ownership must be proven again by session events after a
        // restart. Preserve route IDs and queues, but never trust stale panes.
        state.deactivate_all_sessions(now());

        let mut configuration = ClinchSettings::as_ref(ctx).imessage().clone();
        let recalibration_required = requires_recalibration(&configuration, &state);
        if recalibration_required {
            log::warn!(
                "Local iMessage cursor state is unavailable; requiring calibration before watching Messages"
            );
            configuration.enabled = false;
            configuration.setup_complete = false;
            configuration.chat_id = None;
            configuration.chat_guid = None;
            state.reset_conversation_state();
        }
        state.globally_enabled = configuration.enabled && configuration.setup_complete;
        state.notifications_enabled_by_default =
            configuration.notifications_enabled_by_default;
        let status = status_for_configuration(&configuration, state.pending_calibration.is_some());

        ctx.subscribe_to_model(
            &CLIAgentSessionsModel::handle(ctx),
            |coordinator, _, event, ctx| coordinator.handle_session_event(event, ctx),
        );
        ctx.subscribe_to_model(&ClinchSettings::handle(ctx), |coordinator, _, _, ctx| {
            coordinator.sync_configuration(ctx)
        });

        let mut coordinator = Self {
            status,
            configuration,
            state,
            store,
            bridge: None,
            view_sessions: HashMap::new(),
            session_views: HashMap::new(),
            outbound_jobs: VecDeque::new(),
            mobile_submissions_in_flight: HashSet::new(),
            send_in_flight: false,
            test_status: IMessageTestStatus::Idle,
            current_send_intent_id: None,
            bridge_generation: 0,
            bridge_restart_attempts: 0,
            bridge_restart_scheduled: false,
            expiration_generation: 0,
        };
        coordinator.expire_local_state(ctx);
        coordinator.persist_state();
        if recalibration_required {
            coordinator.save_configuration(ctx);
        }
        if coordinator.should_run_bridge() {
            coordinator.start_bridge(coordinator.should_send_calibration(), ctx);
        }
        coordinator
    }

    pub(crate) fn status(&self) -> &IMessageConnectionStatus {
        &self.status
    }

    pub(crate) fn configuration(&self) -> &IMessageConfiguration {
        &self.configuration
    }

    pub(crate) fn test_status(&self) -> IMessageTestStatus {
        self.test_status
    }

    pub(crate) fn session_notifications_enabled(
        &self,
        terminal_view_id: EntityId,
    ) -> Option<bool> {
        let key = self.view_sessions.get(&terminal_view_id)?;
        let route = self.state.route_for_key(key)?;
        Some(route.notifications_enabled(self.state.notifications_enabled_by_default))
    }

    pub(crate) fn route_id_for_view(&self, terminal_view_id: EntityId) -> Option<&MobileRouteId> {
        let key = self.view_sessions.get(&terminal_view_id)?;
        Some(&self.state.route_for_key(key)?.id)
    }

    pub(crate) fn has_supported_session(&self, terminal_view_id: EntityId) -> bool {
        self.view_sessions.contains_key(&terminal_view_id)
    }

    pub(crate) fn begin_setup(&mut self, recipient: String, ctx: &mut ModelContext<Self>) {
        let recipient = recipient.trim().to_owned();
        if !valid_recipient(&recipient) {
            self.set_status(IMessageConnectionStatus::Error, ctx);
            return;
        }
        self.configuration = configuration_for_setup(
            recipient,
            self.configuration.notifications_enabled_by_default,
        );
        self.outbound_jobs.clear();
        self.test_status = IMessageTestStatus::Idle;
        self.state.globally_enabled = false;
        self.state.reset_conversation_state();
        self.bridge_restart_attempts = 0;
        self.schedule_expiration_check(ctx);
        self.save_configuration(ctx);
        self.persist_state();
        self.start_bridge(true, ctx);
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool, ctx: &mut ModelContext<Self>) {
        if self.configuration.enabled == enabled {
            return;
        }
        self.configuration.enabled = enabled;
        self.state.globally_enabled = enabled && self.configuration.setup_complete;
        self.save_configuration(ctx);
        self.persist_state();
        if !self.configuration.setup_complete {
            if self.bridge.is_none() && self.should_run_bridge() {
                self.start_bridge(self.should_send_calibration(), ctx);
            } else {
                ctx.emit(IMessageCoordinatorEvent::Changed);
            }
            return;
        }
        if enabled {
            self.start_bridge(false, ctx);
        } else {
            self.stop_bridge();
            self.outbound_jobs.clear();
            self.test_status = IMessageTestStatus::Idle;
            self.set_status(IMessageConnectionStatus::Disabled, ctx);
        }
    }

    pub(crate) fn send_test_message(&mut self, ctx: &mut ModelContext<Self>) {
        if matches!(self.test_status, IMessageTestStatus::Sending) {
            return;
        }
        if !self.configuration.setup_complete
            || !self.configuration.enabled
            || !matches!(self.status, IMessageConnectionStatus::Connected)
        {
            self.set_test_status(IMessageTestStatus::Failed, ctx);
            return;
        }
        self.outbound_jobs.push_back(OutboundJob {
            route_id: None,
            parts: VecDeque::from([
                "Clinch test message. Your iMessage connection is working.".to_owned(),
            ]),
            drain_after_send: false,
            kind: OutboundJobKind::ConnectionTest,
        });
        self.set_test_status(IMessageTestStatus::Sending, ctx);
        self.start_next_send(ctx);
    }

    pub(crate) fn set_notifications_enabled_by_default(
        &mut self,
        enabled: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.configuration.notifications_enabled_by_default == enabled {
            return;
        }
        self.configuration.notifications_enabled_by_default = enabled;
        self.state.notifications_enabled_by_default = enabled;

        let disabled_route_ids = self
            .state
            .routes
            .iter()
            .filter(|route| route.active && !route.notifications_enabled(enabled))
            .map(|route| route.id.clone())
            .collect::<Vec<_>>();
        self.cancel_pending_completion_jobs(&disabled_route_ids);
        let cancelled_count = disabled_route_ids
            .iter()
            .map(|route_id| self.state.take_queued_for_route(route_id).len())
            .sum::<usize>();

        self.save_configuration(ctx);
        self.persist_state();
        self.schedule_expiration_check(ctx);
        if cancelled_count > 0 {
            self.enqueue_system_message(format!(
                "{cancelled_count} queued phone {} cancelled because notifications are off by default.",
                if cancelled_count == 1 { "reply was" } else { "replies were" }
            ));
        }
        ctx.emit(IMessageCoordinatorEvent::Changed);
        self.start_next_send(ctx);
    }

    pub(crate) fn disconnect(&mut self, ctx: &mut ModelContext<Self>) {
        self.stop_bridge();
        let notifications_enabled_by_default =
            self.configuration.notifications_enabled_by_default;
        self.configuration = IMessageConfiguration {
            notifications_enabled_by_default,
            ..IMessageConfiguration::default()
        };
        self.state = RouteState {
            notifications_enabled_by_default,
            ..RouteState::default()
        };
        self.outbound_jobs.clear();
        self.test_status = IMessageTestStatus::Idle;
        self.mobile_submissions_in_flight.clear();
        self.send_in_flight = false;
        self.current_send_intent_id = None;
        self.schedule_expiration_check(ctx);
        self.save_configuration(ctx);
        if let Err(error) = self.store.clear() {
            log::warn!("Could not clear local iMessage routing state: {error:#}");
        }
        self.set_status(IMessageConnectionStatus::SetupRequired, ctx);
    }

    pub(crate) fn refresh_health(&mut self, ctx: &mut ModelContext<Self>) {
        self.bridge_restart_attempts = 0;
        if !self.should_run_bridge() {
            self.stop_bridge();
            self.set_status(
                status_for_configuration(
                    &self.configuration,
                    self.state.pending_calibration.is_some(),
                ),
                ctx,
            );
            return;
        }
        if self.bridge.is_none() {
            self.start_bridge(self.should_send_calibration(), ctx);
            return;
        }
        let Some(bridge) = self.bridge.clone() else {
            return;
        };
        let generation = self.bridge_generation;
        ctx.spawn(
            async move { bridge.request(BridgeCommand::Health).await },
            move |coordinator, response, ctx| {
                if coordinator.bridge_generation == generation {
                    coordinator.handle_health_response(response, ctx);
                }
            },
        );
    }

    pub(crate) fn toggle_session(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(key) = self.view_sessions.get(&terminal_view_id).cloned() else {
            return;
        };
        if self.state.route_for_key(&key).is_none() {
            self.register_mapped_sessions(ctx);
        }
        let currently_enabled = self
            .state
            .route_for_key(&key)
            .is_some_and(|route| {
                route.notifications_enabled(self.state.notifications_enabled_by_default)
            });
        let cancelled =
            self.state
                .set_notifications_enabled(&key, !currently_enabled, now());
        if currently_enabled {
            let route_ids = self
                .state
                .route_for_key(&key)
                .map(|route| vec![route.id.clone()])
                .unwrap_or_default();
            self.cancel_pending_completion_jobs(&route_ids);
        }
        self.persist_state();
        self.schedule_expiration_check(ctx);
        if !cancelled.is_empty() {
            self.enqueue_system_message(format!(
                "{} queued phone {} cancelled because Get notified was turned off.",
                cancelled.len(),
                if cancelled.len() == 1 {
                    "reply was"
                } else {
                    "replies were"
                }
            ));
        }
        ctx.emit(IMessageCoordinatorEvent::SessionChanged { terminal_view_id });
        ctx.emit(IMessageCoordinatorEvent::Changed);
        self.start_next_send(ctx);
    }

    fn sync_configuration(&mut self, ctx: &mut ModelContext<Self>) {
        let configuration = ClinchSettings::as_ref(ctx).imessage().clone();
        if configuration == self.configuration {
            return;
        }
        let was_running = self.should_run_bridge();
        self.configuration = configuration;
        self.state.globally_enabled =
            self.configuration.enabled && self.configuration.setup_complete;
        self.state.notifications_enabled_by_default =
            self.configuration.notifications_enabled_by_default;
        self.persist_state();
        let should_run = self.should_run_bridge();
        if should_run && (!was_running || self.bridge.is_none()) {
            self.start_bridge(self.should_send_calibration(), ctx);
        } else if !should_run {
            self.stop_bridge();
            self.outbound_jobs.clear();
            let status = status_for_configuration(
                &self.configuration,
                self.state.pending_calibration.is_some(),
            );
            self.set_status(status, ctx);
        } else {
            ctx.emit(IMessageCoordinatorEvent::Changed);
        }
    }

    fn should_run_bridge(&self) -> bool {
        (self.configuration.enabled && self.configuration.setup_complete)
            || self.state.pending_calibration.is_some()
            || (!self.configuration.setup_complete
                && !self.configuration.recipient.trim().is_empty())
    }

    fn start_bridge(&mut self, send_calibration: bool, ctx: &mut ModelContext<Self>) {
        self.stop_bridge();
        let Some(path) = bridge_executable_path().filter(|path| path.is_file()) else {
            self.set_status(IMessageConnectionStatus::Error, ctx);
            return;
        };
        let bridge = match IMessageBridge::spawn(&path) {
            Ok(bridge) => Arc::new(bridge),
            Err(error) => {
                log::warn!("Could not start the local Messages bridge: {error:#}");
                self.set_status(IMessageConnectionStatus::Error, ctx);
                self.schedule_bridge_restart(ctx);
                return;
            }
        };
        let events = bridge.events();
        self.bridge = Some(bridge.clone());
        let generation = self.bridge_generation;
        self.set_status(IMessageConnectionStatus::Connecting, ctx);
        self.receive_next_bridge_event(events, generation, ctx);

        let command = BridgeCommand::Configure {
            recipient: self.configuration.recipient.clone(),
            chat_id: self.configuration.chat_id,
            chat_guid: self.configuration.chat_guid.clone(),
        };
        ctx.spawn(
            async move { bridge.request(command).await },
            move |coordinator, response, ctx| {
                if coordinator.bridge_generation == generation {
                    coordinator.handle_configure_response(response, send_calibration, ctx)
                }
            },
        );
    }

    fn stop_bridge(&mut self) {
        self.bridge_generation = self.bridge_generation.wrapping_add(1);
        self.bridge_restart_scheduled = false;
        if self.send_in_flight {
            if let Some(job) = self.outbound_jobs.front_mut() {
                // A killed helper leaves delivery indeterminate. Never resend
                // the same text automatically and risk a duplicate iMessage.
                job.parts.clear();
            }
        }
        if let Some(bridge) = self.bridge.take() {
            bridge.terminate();
        }
        self.send_in_flight = false;
        self.current_send_intent_id = None;
    }

    fn should_send_calibration(&self) -> bool {
        !self.configuration.setup_complete
            && self.state.pending_calibration.is_none()
            && !self.configuration.recipient.trim().is_empty()
    }

    fn receive_next_bridge_event(
        &mut self,
        events: async_channel::Receiver<IMessageBridgeEvent>,
        generation: u64,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.spawn(
            {
                let events = events.clone();
                async move { events.recv().await }
            },
            move |coordinator, event, ctx| {
                if coordinator.bridge_generation != generation {
                    return;
                }
                match event {
                    Ok(event) => coordinator.handle_bridge_event(event, ctx),
                    Err(_) => coordinator.handle_bridge_exit(ctx),
                }
                if coordinator.bridge.is_some() && coordinator.bridge_generation == generation {
                    coordinator.receive_next_bridge_event(events, generation, ctx);
                }
            },
        );
    }

    fn handle_configure_response(
        &mut self,
        response: anyhow::Result<BridgeResponse>,
        send_calibration: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(result) = self.successful_result(response, ctx) else {
            return;
        };
        let BridgeResult::Configured {
            chat_id, chat_guid, ..
        } = result
        else {
            self.set_status(IMessageConnectionStatus::Error, ctx);
            return;
        };
        if chat_id.is_some() {
            self.configuration.chat_id = chat_id;
        }
        if chat_guid.is_some() {
            self.configuration.chat_guid = chat_guid;
        }
        self.save_configuration(ctx);

        if send_calibration {
            self.send_calibration(ctx);
        } else if let Some(chat_id) = self.configuration.chat_id {
            self.start_watch(chat_id, self.state.last_row_id, ctx);
        } else {
            self.set_status(IMessageConnectionStatus::SetupRequired, ctx);
        }
    }

    fn send_calibration(&mut self, ctx: &mut ModelContext<Self>) {
        let code = Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(6)
            .collect::<String>()
            .to_ascii_uppercase();
        let expected_reply = format!("CLINCH {code}");
        let text = format!(
            "Clinch setup test. Reply exactly with:\n\n{expected_reply}\n\nNo iPhone app is required."
        );
        // Persist the challenge before invoking Apple Events. If the helper or
        // app exits after Messages accepts the send but before the helper can
        // correlate its GUID, setup resumes by watching for this reply instead
        // of automatically sending a duplicate calibration message.
        self.state.pending_calibration = Some(PendingCalibration {
            expected_reply,
            sent_guid: String::new(),
            created_at: now(),
        });
        self.persist_state();
        self.set_status(IMessageConnectionStatus::AwaitingCalibrationReply, ctx);
        let Some(bridge) = self.bridge.clone() else {
            self.set_status(IMessageConnectionStatus::Error, ctx);
            return;
        };
        let generation = self.bridge_generation;
        ctx.spawn(
            async move {
                bridge
                    .request(BridgeCommand::Send {
                        text,
                        route_id: None,
                    })
                    .await
            },
            move |coordinator, response, ctx| {
                if coordinator.bridge_generation != generation {
                    return;
                }
                coordinator.finish_calibration_send(response, ctx);
            },
        );
    }

    fn finish_calibration_send(
        &mut self,
        response: anyhow::Result<BridgeResponse>,
        ctx: &mut ModelContext<Self>,
    ) {
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                log::warn!("Local Messages calibration request failed: {error:#}");
                self.handle_bridge_exit(ctx);
                return;
            }
        };
        if !response.ok {
            let code = response.error.as_ref().map(|error| error.code.as_str());
            let status = code
                .map(status_for_bridge_error_code)
                .unwrap_or(IMessageConnectionStatus::Error);

            // These failures happen before Messages accepts a send, so a user
            // can grant the permission and safely retry. Every other failure
            // is delivery-indeterminate and keeps the pending challenge.
            if matches!(
                code,
                Some(
                    "automation_required"
                        | "full_disk_access_required"
                        | "messages_sign_in_required"
                )
            ) {
                self.state.pending_calibration = None;
                self.persist_state();
            }
            self.set_status(status.clone(), ctx);
            if matches!(code, Some("sent_message_not_observed")) {
                if let Some(chat_id) = self.configuration.chat_id {
                    self.start_watch(chat_id, self.state.last_row_id, ctx);
                }
            } else if matches!(status, IMessageConnectionStatus::Error) {
                self.handle_bridge_exit(ctx);
            }
            return;
        }

        let Some(BridgeResult::Sent {
            guid,
            row_id,
            chat_id,
            chat_guid,
        }) = response.result
        else {
            self.handle_bridge_exit(ctx);
            return;
        };
        self.configuration.chat_id = Some(chat_id);
        if chat_guid.is_some() {
            self.configuration.chat_guid = chat_guid;
        }
        self.state.record_system_outbound_guid(&guid, now());
        if let Some(calibration) = self.state.pending_calibration.as_mut() {
            calibration.sent_guid = guid;
        }
        self.state.last_row_id = self.state.last_row_id.max(row_id);
        self.save_configuration(ctx);
        self.persist_state();
        self.set_status(IMessageConnectionStatus::AwaitingCalibrationReply, ctx);
        self.start_watch(chat_id, row_id, ctx);
    }

    fn start_watch(&mut self, chat_id: i64, after_row_id: i64, ctx: &mut ModelContext<Self>) {
        let Some(bridge) = self.bridge.clone() else {
            return;
        };
        let generation = self.bridge_generation;
        ctx.spawn(
            async move {
                bridge
                    .request(BridgeCommand::StartWatch {
                        chat_id,
                        after_row_id,
                    })
                    .await
            },
            move |coordinator, response, ctx| {
                if coordinator.bridge_generation != generation {
                    return;
                }
                let Some(result) = coordinator.successful_result(response, ctx) else {
                    return;
                };
                if !matches!(result, BridgeResult::Watching) {
                    coordinator.set_status(IMessageConnectionStatus::Error, ctx);
                    return;
                }
                let status = if coordinator.state.pending_calibration.is_some() {
                    IMessageConnectionStatus::AwaitingCalibrationReply
                } else {
                    IMessageConnectionStatus::Connected
                };
                coordinator.bridge_restart_attempts = 0;
                coordinator.set_status(status, ctx);
                coordinator.expire_local_state(ctx);
                coordinator.start_next_send(ctx);
            },
        );
    }

    fn handle_health_response(
        &mut self,
        response: anyhow::Result<BridgeResponse>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(result) = self.successful_result(response, ctx) else {
            return;
        };
        let BridgeResult::Health {
            database_readable,
            automation_authorized,
        } = result
        else {
            return;
        };
        let status = if !database_readable {
            IMessageConnectionStatus::Paused(IMessagePermission::FullDiskAccess)
        } else if !automation_authorized {
            IMessageConnectionStatus::Paused(IMessagePermission::Automation)
        } else {
            // A health response does not recreate a watcher that stopped when
            // Full Disk Access was revoked. Reconfigure a fresh helper so
            // restoring permission actually resumes from the persisted cursor.
            self.start_bridge(self.should_send_calibration(), ctx);
            return;
        };
        self.set_status(status, ctx);
    }

    fn handle_bridge_event(&mut self, event: IMessageBridgeEvent, ctx: &mut ModelContext<Self>) {
        match event {
            IMessageBridgeEvent::Message(event) => self.handle_protocol_event(event, ctx),
            IMessageBridgeEvent::Exited | IMessageBridgeEvent::ProtocolError => {
                self.handle_bridge_exit(ctx);
            }
        }
    }

    fn handle_protocol_event(&mut self, event: BridgeEvent, ctx: &mut ModelContext<Self>) {
        match event {
            BridgeEvent::Incoming { version, message } => {
                if version != BRIDGE_PROTOCOL_VERSION {
                    self.set_status(IMessageConnectionStatus::Error, ctx);
                    return;
                }
                self.handle_incoming(message, ctx);
            }
            BridgeEvent::PermissionRequired {
                version,
                permission,
            } if version == BRIDGE_PROTOCOL_VERSION => {
                self.set_status(IMessageConnectionStatus::Paused(permission.into()), ctx);
            }
            BridgeEvent::DeliveryFailed { version, code }
            | BridgeEvent::WatchFailed { version, code }
                if version == BRIDGE_PROTOCOL_VERSION =>
            {
                let status = status_for_bridge_error_code(&code);
                let retry = matches!(status, IMessageConnectionStatus::Error);
                self.set_status(status, ctx);
                if retry {
                    self.handle_bridge_exit(ctx);
                }
            }
            _ => self.set_status(IMessageConnectionStatus::Error, ctx),
        }
    }

    fn handle_bridge_exit(&mut self, ctx: &mut ModelContext<Self>) {
        self.bridge_generation = self.bridge_generation.wrapping_add(1);
        if self.send_in_flight {
            if let Some(job) = self.outbound_jobs.front_mut() {
                job.parts.clear();
            }
        }
        if let Some(bridge) = self.bridge.take() {
            bridge.terminate();
        }
        self.send_in_flight = false;
        self.current_send_intent_id = None;
        self.set_status(IMessageConnectionStatus::Error, ctx);
        self.schedule_bridge_restart(ctx);
    }

    fn schedule_bridge_restart(&mut self, ctx: &mut ModelContext<Self>) {
        const DELAYS: [u64; 5] = [1, 2, 5, 15, 30];
        if !self.should_run_bridge() || self.bridge_restart_scheduled {
            return;
        }
        let Some(delay) = DELAYS.get(self.bridge_restart_attempts).copied() else {
            self.set_status(IMessageConnectionStatus::Error, ctx);
            return;
        };
        self.bridge_restart_attempts += 1;
        self.bridge_restart_scheduled = true;
        let generation = self.bridge_generation;
        ctx.spawn(
            Timer::after(Duration::from_secs(delay)),
            move |coordinator, _, ctx| {
                if coordinator.bridge_generation != generation {
                    return;
                }
                coordinator.bridge_restart_scheduled = false;
                if coordinator.should_run_bridge() {
                    coordinator.start_bridge(coordinator.should_send_calibration(), ctx);
                }
            },
        );
    }

    fn handle_incoming(
        &mut self,
        mut message: super::domain::IncomingMessage,
        ctx: &mut ModelContext<Self>,
    ) {
        message.text = sanitize_incoming_text(&message.text);
        self.expire_local_state(ctx);
        if let Some(calibration) = self.state.pending_calibration.clone() {
            self.state.last_row_id = self.state.last_row_id.max(message.row_id);
            if message.is_supported_text()
                && message.guid != calibration.sent_guid
                && message
                    .text
                    .trim()
                    .eq_ignore_ascii_case(&calibration.expected_reply)
            {
                self.state.mark_processed(message.guid, now());
                self.state.pending_calibration = None;
                self.configuration.setup_complete = true;
                self.state.globally_enabled = self.configuration.enabled;
                self.register_mapped_sessions(ctx);
                self.save_configuration(ctx);
                self.persist_state();
                if self.configuration.enabled {
                    self.set_status(IMessageConnectionStatus::Connected, ctx);
                    self.enqueue_system_message(
                        "Clinch is connected. Finished Codex and Claude sessions will message you by default."
                            .to_owned(),
                    );
                    self.start_next_send(ctx);
                } else {
                    self.set_status(IMessageConnectionStatus::Disabled, ctx);
                }
            } else if message.guid != calibration.sent_guid {
                self.state.mark_processed(message.guid, now());
                self.persist_state();
            }
            return;
        }

        let decision = self.state.route_incoming(&message, now());
        if !matches!(decision, RouteDecision::Duplicate) {
            self.state.mark_processed(message.guid.clone(), now());
        }
        match decision {
            RouteDecision::Deliver { route_id, text } => {
                self.deliver_or_queue(message.guid, route_id, text, ctx)
            }
            RouteDecision::Ambiguous {
                candidate_route_ids,
                ..
            } => {
                let routes = candidate_route_ids
                    .iter()
                    .filter_map(|id| self.state.route_by_id(id))
                    .map(|route| format!("{} — {}", route.id, route.label))
                    .collect::<Vec<_>>()
                    .join("\n");
                self.enqueue_system_message(format!(
                    "Which agent session should receive that reply? Reply with only its code:\n\n{routes}"
                ));
            }
            RouteDecision::UnknownRoute(route_id) => self.enqueue_system_message(format!(
                "{route_id} is not an active Clinch route. Open Clinch or use a code from a recent completion message."
            )),
            RouteDecision::NoPendingSelection(route_id) => self.enqueue_system_message(format!(
                "There is no retained reply waiting for {route_id}. It may have expired; send the original message again."
            )),
            RouteDecision::NoEligibleRoute => self.enqueue_system_message(
                "No enabled live Clinch agent session can receive that reply.".to_owned(),
            ),
            RouteDecision::Ignore | RouteDecision::Duplicate => {}
        }
        self.persist_state();
        self.schedule_expiration_check(ctx);
        self.start_next_send(ctx);
    }

    fn deliver_or_queue(
        &mut self,
        source_guid: String,
        route_id: MobileRouteId,
        text: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(route) = self.state.route_by_id(&route_id).cloned() else {
            self.enqueue_system_message("That Clinch route is no longer available.".to_owned());
            return;
        };
        let Some(view_id) = self.session_views.get(&route.key).copied() else {
            self.enqueue_system_message(format!(
                "{} is no longer attached to a live agent session.",
                route.id
            ));
            return;
        };
        let status = CLIAgentSessionsModel::as_ref(ctx)
            .session(view_id)
            .filter(|session| session_key(session.session_key()) == Some(route.key.clone()))
            .map(|session| session.status.clone());
        let has_backlog = self.state.has_queued_for_route(&route.id)
            || self.mobile_submissions_in_flight.contains(&route.key)
            || self.has_pending_completion_drain(&route.id);
        let submit_immediately = can_submit_mobile_reply_immediately(status.as_ref(), has_backlog);
        match status {
            Some(CLIAgentSessionStatus::Success) if submit_immediately => {
                if submit_to_exact_session(view_id, &route.key, text, ctx) {
                    self.mobile_submissions_in_flight.insert(route.key);
                } else {
                    self.enqueue_system_message(format!(
                        "{} could not receive the reply because its agent process is no longer active.",
                        route.id
                    ));
                }
            }
            Some(CLIAgentSessionStatus::Success)
            | Some(CLIAgentSessionStatus::InProgress)
            | Some(CLIAgentSessionStatus::Blocked { .. }) => {
                let _ = self
                    .state
                    .enqueue_reply(source_guid, route.id.clone(), text, now());
            }
            None => self.enqueue_system_message(format!(
                "{} could not be matched to the original agent session.",
                route.id
            )),
        }
    }

    fn has_pending_completion_drain(&self, route_id: &MobileRouteId) -> bool {
        self.outbound_jobs.iter().any(|job| {
            job.drain_after_send && job.route_id.as_ref().is_some_and(|id| id == route_id)
        })
    }

    fn handle_session_event(
        &mut self,
        event: &CLIAgentSessionsModelEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        self.expire_local_state(ctx);
        let view_id = event.terminal_view_id();
        if matches!(event, CLIAgentSessionsModelEvent::Ended { .. }) {
            if let Some(key) = self.view_sessions.remove(&view_id) {
                self.session_views.remove(&key);
                self.mobile_submissions_in_flight.remove(&key);
                let cancelled = self.state.retire_session(&key, now());
                if !cancelled.is_empty() {
                    self.enqueue_system_message(format!(
                        "{} queued phone {} cancelled because the agent session ended.",
                        cancelled.len(),
                        if cancelled.len() == 1 {
                            "reply was"
                        } else {
                            "replies were"
                        }
                    ));
                }
            }
            self.persist_state();
            self.schedule_expiration_check(ctx);
            ctx.emit(IMessageCoordinatorEvent::SessionChanged {
                terminal_view_id: view_id,
            });
            self.start_next_send(ctx);
            return;
        }

        self.reconcile_view_session(view_id, ctx);

        let CLIAgentSessionsModelEvent::StatusChanged {
            status,
            session_context,
            ..
        } = event
        else {
            return;
        };
        if !matches!(status, CLIAgentSessionStatus::Success) {
            return;
        }
        let Some(key) = self.view_sessions.get(&view_id).cloned() else {
            return;
        };
        self.mobile_submissions_in_flight.remove(&key);
        let Some(route) = self.state.route_for_key(&key).cloned() else {
            return;
        };
        if !route.is_eligible(
            self.state.globally_enabled,
            self.state.notifications_enabled_by_default,
        ) {
            return;
        }

        let label = session_context
            .project
            .as_deref()
            .or(session_context.cwd.as_deref())
            .unwrap_or("Agent session");
        let parts = format_completion_messages(
            &route.id,
            key.provider,
            label,
            session_context.response.as_deref(),
        );
        self.outbound_jobs.push_back(OutboundJob {
            route_id: Some(route.id),
            parts: parts.into(),
            drain_after_send: true,
            kind: OutboundJobKind::Message,
        });
        self.start_next_send(ctx);
    }

    fn reconcile_view_session(&mut self, view_id: EntityId, ctx: &mut ModelContext<Self>) {
        let current = CLIAgentSessionsModel::as_ref(ctx)
            .session(view_id)
            .and_then(|session| session_key(session.session_key()));
        if self.view_sessions.get(&view_id) == current.as_ref() {
            return;
        }
        if let Some(previous) = self.view_sessions.remove(&view_id) {
            self.session_views.remove(&previous);
            self.mobile_submissions_in_flight.remove(&previous);
            let cancelled = self.state.retire_session(&previous, now());
            if !cancelled.is_empty() {
                self.enqueue_system_message(
                    "Queued phone replies were cancelled because the agent session changed identity."
                        .to_owned(),
                );
            }
        }
        if let Some(key) = current {
            let label = CLIAgentSessionsModel::as_ref(ctx)
                .session(view_id)
                .and_then(|session| {
                    session
                        .session_context
                        .project
                        .clone()
                        .or_else(|| session.session_context.cwd.clone())
                })
                .unwrap_or_else(|| format!("{} session", key.provider.display_name()));
            self.state.register_session(key.clone(), label, now());
            self.view_sessions.insert(view_id, key.clone());
            self.session_views.insert(key, view_id);
        }
        self.persist_state();
        ctx.emit(IMessageCoordinatorEvent::SessionChanged {
            terminal_view_id: view_id,
        });
        ctx.emit(IMessageCoordinatorEvent::Changed);
    }

    fn register_mapped_sessions(&mut self, ctx: &AppContext) {
        let mapped = self
            .view_sessions
            .iter()
            .map(|(view_id, key)| (*view_id, key.clone()))
            .collect::<Vec<_>>();
        for (view_id, key) in mapped {
            let Some(session) = CLIAgentSessionsModel::as_ref(ctx)
                .session(view_id)
                .filter(|session| session_key(session.session_key()) == Some(key.clone()))
            else {
                continue;
            };
            let label = session
                .session_context
                .project
                .clone()
                .or_else(|| session.session_context.cwd.clone())
                .unwrap_or_else(|| format!("{} session", key.provider.display_name()));
            self.state.register_session(key, label, now());
        }
    }

    fn enqueue_system_message(&mut self, text: String) {
        if !self.configuration.setup_complete
            || !self.configuration.enabled
            || text.trim().is_empty()
        {
            return;
        }
        self.outbound_jobs.push_back(OutboundJob {
            route_id: None,
            parts: VecDeque::from([text]),
            drain_after_send: false,
            kind: OutboundJobKind::Message,
        });
    }

    fn expire_local_state(&mut self, ctx: &mut ModelContext<Self>) {
        let expired = self.state.take_expired(now());
        if !expired.pending_selections.is_empty() {
            self.enqueue_system_message(format!(
                "{} retained phone {} after 10 minutes. Send the original text again to choose a route.",
                expired.pending_selections.len(),
                if expired.pending_selections.len() == 1 {
                    "reply expired"
                } else {
                    "replies expired"
                }
            ));
        }
        if !expired.queued_replies.is_empty() {
            let mut route_ids = expired
                .queued_replies
                .iter()
                .map(|reply| reply.route_id.to_string())
                .collect::<Vec<_>>();
            route_ids.sort_unstable();
            route_ids.dedup();
            self.enqueue_system_message(format!(
                "{} queued phone {} after 24 hours for {}.",
                expired.queued_replies.len(),
                if expired.queued_replies.len() == 1 {
                    "reply expired"
                } else {
                    "replies expired"
                },
                route_ids.join(", ")
            ));
        }
        if expired.state_changed {
            self.persist_state();
        }
        self.schedule_expiration_check(ctx);
    }

    fn schedule_expiration_check(&mut self, ctx: &mut ModelContext<Self>) {
        self.expiration_generation = self.expiration_generation.wrapping_add(1);
        let generation = self.expiration_generation;
        let Some(expires_at) = self.state.next_expiration_at() else {
            return;
        };
        let delay_seconds = expires_at.saturating_sub(now()).max(0) as u64;
        ctx.spawn(
            Timer::after(Duration::from_secs(delay_seconds)),
            move |coordinator, _, ctx| {
                if coordinator.expiration_generation != generation {
                    return;
                }
                coordinator.expire_local_state(ctx);
                coordinator.start_next_send(ctx);
            },
        );
    }

    fn start_next_send(&mut self, ctx: &mut ModelContext<Self>) {
        if self.send_in_flight || !matches!(self.status, IMessageConnectionStatus::Connected) {
            return;
        }
        loop {
            while self
                .outbound_jobs
                .front()
                .is_some_and(|job| job.parts.is_empty())
            {
                let completed = self.outbound_jobs.pop_front().unwrap();
                if completed.drain_after_send {
                    if let Some(route_id) = completed.route_id {
                        self.submit_next_queued(&route_id, ctx);
                    }
                }
            }
            let route_is_disabled = self
                .outbound_jobs
                .front()
                .and_then(|job| job.route_id.as_ref())
                .is_some_and(|route_id| {
                    !self.state.route_by_id(route_id).is_some_and(|route| {
                        route.is_eligible(
                            self.state.globally_enabled,
                            self.state.notifications_enabled_by_default,
                        )
                    })
                });
            if route_is_disabled {
                self.outbound_jobs.pop_front();
                continue;
            }
            break;
        }
        let Some(job) = self.outbound_jobs.front() else {
            return;
        };
        let Some(text) = job.parts.front().cloned() else {
            return;
        };
        let route_id = job.route_id.clone();
        let Some(bridge) = self.bridge.clone() else {
            self.set_status(IMessageConnectionStatus::Error, ctx);
            return;
        };
        let generation = self.bridge_generation;
        let intent_id = self
            .state
            .record_outbound_intent(&text, route_id.clone(), now());
        self.current_send_intent_id = Some(intent_id);
        self.persist_state();
        self.schedule_expiration_check(ctx);
        self.send_in_flight = true;
        ctx.spawn(
            async move {
                bridge
                    .request(BridgeCommand::Send {
                        text,
                        route_id: route_id.as_ref().map(ToString::to_string),
                    })
                    .await
            },
            move |coordinator, response, ctx| {
                if coordinator.bridge_generation == generation {
                    coordinator.finish_send(response, ctx);
                }
            },
        );
    }

    fn finish_send(
        &mut self,
        response: anyhow::Result<BridgeResponse>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.send_in_flight = false;
        let intent_id = self.current_send_intent_id.take();
        let job_kind = self.outbound_jobs.front().map(|job| job.kind);
        let result = self.successful_result(response, ctx);
        let sent = matches!(&result, Some(BridgeResult::Sent { .. }));
        match result {
            Some(BridgeResult::Sent {
                guid,
                chat_id,
                chat_guid,
                ..
            }) => {
                if let Some(intent_id) = intent_id.as_deref() {
                    self.state.resolve_outbound_intent(intent_id);
                }
                self.configuration.chat_id = Some(chat_id);
                if chat_guid.is_some() {
                    self.configuration.chat_guid = chat_guid;
                }
                if let Some(job) = self.outbound_jobs.front_mut() {
                    if let Some(route_id) = job.route_id.clone() {
                        self.state.record_outbound_guid(guid, route_id, now());
                    } else {
                        self.state.record_system_outbound_guid(guid, now());
                    }
                    job.parts.pop_front();
                }
                self.save_configuration(ctx);
                self.persist_state();
            }
            _ => {
                if let Some(job) = self.outbound_jobs.front_mut() {
                    job.parts.clear();
                }
            }
        }
        if matches!(job_kind, Some(OutboundJobKind::ConnectionTest)) {
            self.set_test_status(
                if sent {
                    IMessageTestStatus::Sent
                } else {
                    IMessageTestStatus::Failed
                },
                ctx,
            );
        }
        self.start_next_send(ctx);
    }

    fn cancel_pending_completion_jobs(&mut self, route_ids: &[MobileRouteId]) {
        if route_ids.is_empty() {
            return;
        }
        let disabled = route_ids.iter().collect::<HashSet<_>>();
        let mut jobs = std::mem::take(&mut self.outbound_jobs);

        if self.send_in_flight {
            if let Some(mut front) = jobs.pop_front() {
                if front
                    .route_id
                    .as_ref()
                    .is_some_and(|route_id| disabled.contains(route_id))
                {
                    // The first part may already have reached Messages and
                    // cannot be recalled. Retain only its correlation slot;
                    // all later parts and the queue drain are cancelled.
                    front.parts.truncate(1);
                    front.drain_after_send = false;
                }
                self.outbound_jobs.push_back(front);
            }
        }

        self.outbound_jobs.extend(jobs.into_iter().filter(|job| {
            !job
                .route_id
                .as_ref()
                .is_some_and(|route_id| disabled.contains(route_id))
        }));
    }

    fn submit_next_queued(&mut self, route_id: &MobileRouteId, ctx: &mut ModelContext<Self>) {
        self.expire_local_state(ctx);
        let Some(route) = self.state.route_by_id(route_id).cloned() else {
            return;
        };
        let Some(view_id) = self.session_views.get(&route.key).copied() else {
            let cancelled = self.state.take_queued_for_route(route_id);
            if !cancelled.is_empty() {
                self.enqueue_system_message(format!(
                    "{} queued phone {} cancelled because {} is no longer attached to a live agent session.",
                    cancelled.len(),
                    if cancelled.len() == 1 {
                        "reply was"
                    } else {
                        "replies were"
                    },
                    route.id
                ));
            }
            self.persist_state();
            self.schedule_expiration_check(ctx);
            return;
        };
        let status = CLIAgentSessionsModel::as_ref(ctx)
            .session(view_id)
            .filter(|session| session_key(session.session_key()) == Some(route.key.clone()))
            .map(|session| session.status.clone());
        match queued_drain_decision(status.as_ref()) {
            QueuedDrainDecision::Wait => return,
            QueuedDrainDecision::Submit => {}
            QueuedDrainDecision::Cancel => {
                let cancelled = self.state.take_queued_for_route(route_id);
                if !cancelled.is_empty() {
                    self.enqueue_system_message(format!(
                        "{} queued phone {} cancelled because the original agent is unavailable.",
                        cancelled.len(),
                        if cancelled.len() == 1 {
                            "reply was"
                        } else {
                            "replies were"
                        }
                    ));
                }
                self.persist_state();
                self.schedule_expiration_check(ctx);
                return;
            }
        }
        let Some(reply) = self.state.pop_next_queued(route_id, now()) else {
            return;
        };
        let submitted = submit_to_exact_session(view_id, &route.key, reply.text, ctx);
        if submitted {
            self.mobile_submissions_in_flight.insert(route.key);
        } else {
            self.enqueue_system_message(format!(
                "{} could not receive a queued reply because the original agent is unavailable.",
                route.id
            ));
        }
        self.persist_state();
        self.schedule_expiration_check(ctx);
    }

    fn successful_result(
        &mut self,
        response: anyhow::Result<BridgeResponse>,
        ctx: &mut ModelContext<Self>,
    ) -> Option<BridgeResult> {
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                log::warn!("Local Messages bridge request failed: {error:#}");
                self.handle_bridge_exit(ctx);
                return None;
            }
        };
        if !response.ok {
            let code = response.error.as_ref().map(|error| error.code.as_str());
            let status = code
                .map(status_for_bridge_error_code)
                .unwrap_or(IMessageConnectionStatus::Error);
            let should_restart = matches!(status, IMessageConnectionStatus::Error)
                && !matches!(code, Some("sent_message_not_observed"));
            self.set_status(status, ctx);
            if should_restart {
                self.handle_bridge_exit(ctx);
            }
            return None;
        }
        response.result
    }

    fn save_configuration(&mut self, ctx: &mut ModelContext<Self>) {
        let value = self.configuration.clone();
        ClinchSettings::handle(ctx).update(ctx, |settings, ctx| {
            if let Err(error) = settings.imessage_configuration.set_value(value, ctx) {
                log::warn!("Could not save the local iMessage configuration: {error:#}");
            }
        });
    }

    fn persist_state(&self) {
        if let Err(error) = self.store.save(&self.state) {
            log::warn!("Could not save local iMessage routing state: {error:#}");
        }
    }

    fn set_status(&mut self, status: IMessageConnectionStatus, ctx: &mut ModelContext<Self>) {
        let test_changed = !matches!(&status, IMessageConnectionStatus::Connected)
            && matches!(self.test_status, IMessageTestStatus::Sending);
        if test_changed {
            self.test_status = IMessageTestStatus::Failed;
        }
        if self.status != status || test_changed {
            self.status = status;
            ctx.emit(IMessageCoordinatorEvent::Changed);
        }
    }

    fn set_test_status(&mut self, status: IMessageTestStatus, ctx: &mut ModelContext<Self>) {
        if self.test_status != status {
            self.test_status = status;
            ctx.emit(IMessageCoordinatorEvent::Changed);
        }
    }
}

fn configuration_for_setup(
    recipient: String,
    notifications_enabled_by_default: bool,
) -> IMessageConfiguration {
    IMessageConfiguration {
        // The connection remains ineffective until calibration completes,
        // but the visible preferences start in their promised default-on
        // state as soon as setup begins.
        enabled: true,
        setup_complete: false,
        notifications_enabled_by_default,
        recipient,
        chat_id: None,
        chat_guid: None,
    }
}

fn status_for_bridge_error_code(code: &str) -> IMessageConnectionStatus {
    match code {
        "full_disk_access_required" => {
            IMessageConnectionStatus::Paused(IMessagePermission::FullDiskAccess)
        }
        "automation_required" => IMessageConnectionStatus::Paused(IMessagePermission::Automation),
        "messages_sign_in_required" => {
            IMessageConnectionStatus::Paused(IMessagePermission::MessagesSignIn)
        }
        _ => IMessageConnectionStatus::Error,
    }
}

fn status_for_configuration(
    configuration: &IMessageConfiguration,
    pending_calibration: bool,
) -> IMessageConnectionStatus {
    if pending_calibration {
        IMessageConnectionStatus::AwaitingCalibrationReply
    } else if !configuration.setup_complete {
        if configuration.recipient.trim().is_empty() {
            IMessageConnectionStatus::SetupRequired
        } else {
            IMessageConnectionStatus::Connecting
        }
    } else if !configuration.enabled {
        IMessageConnectionStatus::Disabled
    } else {
        IMessageConnectionStatus::Connecting
    }
}

fn requires_recalibration(configuration: &IMessageConfiguration, state: &RouteState) -> bool {
    configuration.setup_complete && state.last_row_id <= 0
}

fn valid_recipient(recipient: &str) -> bool {
    let digits = recipient.chars().filter(char::is_ascii_digit).count();
    (7..=15).contains(&digits)
        && recipient.chars().count() <= 32
        && recipient.chars().all(|character| {
            character.is_ascii_digit() || matches!(character, '+' | '-' | '(' | ')' | ' ' | '.')
        })
}

fn now() -> i64 {
    Utc::now().timestamp()
}

fn can_submit_mobile_reply_immediately(
    status: Option<&CLIAgentSessionStatus>,
    has_backlog: bool,
) -> bool {
    !has_backlog && matches!(status, Some(CLIAgentSessionStatus::Success))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QueuedDrainDecision {
    Submit,
    Wait,
    Cancel,
}

fn queued_drain_decision(status: Option<&CLIAgentSessionStatus>) -> QueuedDrainDecision {
    match status {
        Some(CLIAgentSessionStatus::Success) => QueuedDrainDecision::Submit,
        Some(CLIAgentSessionStatus::InProgress) | Some(CLIAgentSessionStatus::Blocked { .. }) => {
            QueuedDrainDecision::Wait
        }
        None => QueuedDrainDecision::Cancel,
    }
}

fn session_key(key: Option<CLIAgentSessionKey>) -> Option<MobileSessionKey> {
    let key = key?;
    MobileSessionKey::new(MobileProvider::from(key.provider), key.session_id)
}

fn cli_session_key(key: &MobileSessionKey) -> CLIAgentSessionKey {
    CLIAgentSessionKey {
        provider: key.provider.into(),
        session_id: key.session_id.clone(),
    }
}

fn terminal_view_by_id(
    terminal_view_id: EntityId,
    app: &AppContext,
) -> Option<ViewHandle<TerminalView>> {
    for window_id in app.window_ids() {
        if let Some(views) = app.views_of_type::<TerminalView>(window_id) {
            if let Some(view) = views.into_iter().find(|view| view.id() == terminal_view_id) {
                return Some(view);
            }
        }
    }
    None
}

fn submit_to_exact_session(
    terminal_view_id: EntityId,
    expected_key: &MobileSessionKey,
    text: String,
    ctx: &mut ModelContext<IMessageCoordinator>,
) -> bool {
    let Some(view) = terminal_view_by_id(terminal_view_id, ctx) else {
        return false;
    };
    let expected_key = cli_session_key(expected_key);
    view.update(ctx, |view, ctx| {
        view.submit_external_imessage_reply(expected_key, text, ctx)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipient_validation_accepts_phone_shapes_only() {
        assert!(valid_recipient("+1 (415) 555-1212"));
        assert!(valid_recipient("4155551212"));
        assert!(!valid_recipient("elliot@example.com"));
        assert!(!valid_recipient("123"));
        assert!(!valid_recipient("1234567890123456"));
    }

    #[test]
    fn starting_setup_defaults_the_connection_on_before_it_becomes_effective() {
        let configuration = configuration_for_setup("+1 415 555 1212".to_owned(), true);

        assert!(configuration.enabled);
        assert!(configuration.notifications_enabled_by_default);
        assert!(!configuration.setup_complete);
        assert_eq!(configuration.recipient, "+1 415 555 1212");
    }

    #[test]
    fn status_follows_setup_before_enablement() {
        assert_eq!(
            status_for_configuration(&IMessageConfiguration::default(), false),
            IMessageConnectionStatus::SetupRequired
        );
        let configured = IMessageConfiguration {
            setup_complete: true,
            ..IMessageConfiguration::default()
        };
        assert_eq!(
            status_for_configuration(&configured, false),
            IMessageConnectionStatus::Disabled
        );
        assert_eq!(
            status_for_configuration(&configured, true),
            IMessageConnectionStatus::AwaitingCalibrationReply
        );
        let configuring = IMessageConfiguration {
            recipient: "+1 415 555 1212".to_owned(),
            ..IMessageConfiguration::default()
        };
        assert_eq!(
            status_for_configuration(&configuring, false),
            IMessageConnectionStatus::Connecting
        );
    }

    #[test]
    fn configured_setup_without_a_cursor_requires_safe_recalibration() {
        let configured = IMessageConfiguration {
            enabled: true,
            setup_complete: true,
            recipient: "+14155551212".to_owned(),
            chat_id: Some(7),
            chat_guid: Some("chat-guid".to_owned()),
            ..IMessageConfiguration::default()
        };
        assert!(requires_recalibration(&configured, &RouteState::default()));

        let state = RouteState {
            last_row_id: 1,
            ..RouteState::default()
        };
        assert!(!requires_recalibration(&configured, &state));
        assert!(!requires_recalibration(
            &IMessageConfiguration::default(),
            &RouteState::default()
        ));
    }

    #[test]
    fn an_idle_status_does_not_overtake_a_local_or_queued_mobile_submission() {
        let success = CLIAgentSessionStatus::Success;
        let in_progress = CLIAgentSessionStatus::InProgress;
        assert!(can_submit_mobile_reply_immediately(Some(&success), false));
        assert!(!can_submit_mobile_reply_immediately(Some(&success), true));
        assert!(!can_submit_mobile_reply_immediately(
            Some(&in_progress),
            false
        ));
        assert!(!can_submit_mobile_reply_immediately(None, false));
    }

    #[test]
    fn a_delayed_completion_drain_never_types_into_a_now_busy_agent() {
        assert_eq!(
            queued_drain_decision(Some(&CLIAgentSessionStatus::Success)),
            QueuedDrainDecision::Submit
        );
        assert_eq!(
            queued_drain_decision(Some(&CLIAgentSessionStatus::InProgress)),
            QueuedDrainDecision::Wait
        );
        assert_eq!(queued_drain_decision(None), QueuedDrainDecision::Cancel);
    }

    #[test]
    fn bridge_permission_error_codes_are_actionable() {
        assert_eq!(
            status_for_bridge_error_code("full_disk_access_required"),
            IMessageConnectionStatus::Paused(IMessagePermission::FullDiskAccess)
        );
        assert_eq!(
            status_for_bridge_error_code("automation_required"),
            IMessageConnectionStatus::Paused(IMessagePermission::Automation)
        );
        assert_eq!(
            status_for_bridge_error_code("messages_sign_in_required"),
            IMessageConnectionStatus::Paused(IMessagePermission::MessagesSignIn)
        );
        assert_eq!(
            status_for_bridge_error_code("schema_changed"),
            IMessageConnectionStatus::Error
        );
    }
}

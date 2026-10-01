//! Bridge between protocol-level control requests and Warp application models.
//!
//! The bridge validates protocol version, selectors, credentials, and settings
//! before routing each supported action to an app-side handler.

use ::local_control::auth::CredentialGrant;
use ::local_control::{
    Action, ActionKind, ControlError, ErrorCode, InstanceId, RequestEnvelope, ResponseEnvelope,
};
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::local_control::handlers::{
    app_state, close, metadata, metadata_config, sections, settings_surfaces, tab_grep, toolbelt,
};
use crate::local_control::permissions::{
    ensure_action_allowed, ensure_feature_enabled, ensure_protocol_version,
};
use crate::local_control::resolver::{validate_action_params, validate_action_target};

/// WarpUI model that executes already-authenticated local-control actions.
pub struct LocalControlBridge {
    pub(super) instance_id: Option<InstanceId>,
    delivery: Option<super::delivery::DeliveryService>,
    observation: Option<super::observation::ObservationService>,
    observing_models: bool,
    observed_views: std::collections::HashSet<warpui::EntityId>,
}

pub(super) enum BridgeReply {
    Immediate(ResponseEnvelope),
    Delivery {
        request_id: uuid::Uuid,
        receiver: super::delivery::PendingReply,
    },
    Read {
        request_id: uuid::Uuid,
        plan: super::conversation::ReadPlan,
    },
}

impl From<ResponseEnvelope> for BridgeReply {
    fn from(response: ResponseEnvelope) -> Self {
        Self::Immediate(response)
    }
}

impl Entity for LocalControlBridge {
    type Event = ();
}

impl SingletonEntity for LocalControlBridge {}

impl LocalControlBridge {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            instance_id: None,
            delivery: None,
            observation: None,
            observing_models: false,
            observed_views: Default::default(),
        }
    }

    pub(super) fn start_observations(&mut self, ctx: &mut ModelContext<Self>) {
        if self.observing_models {
            return;
        }
        self.observing_models = true;
        use crate::terminal::cli_agent_sessions::{
            CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
        };
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), |bridge, _, event, ctx| {
            let kind = match event {
                CLIAgentSessionsModelEvent::Started { .. } => "agent.started",
                CLIAgentSessionsModelEvent::Ended { .. } => "agent.ended",
                CLIAgentSessionsModelEvent::StatusChanged { .. } => "agent.status",
                _ => { bridge.capture(None, ctx); return; }
            };
            let (conversation_id, provider, status) = match event {
                CLIAgentSessionsModelEvent::Started { conversation_id, agent, .. }
                | CLIAgentSessionsModelEvent::Ended { conversation_id, agent, .. } => (conversation_id.clone(), format!("{agent:?}"), None),
                CLIAgentSessionsModelEvent::StatusChanged { agent, session_context, status, .. } =>
                    (session_context.session_id.clone(), format!("{agent:?}"), Some(format!("{status:?}"))),
                _ => unreachable!(),
            };
            bridge.capture(Some(serde_json::json!({"kind": kind, "terminal_id": event.terminal_view_id().to_string(),
                "conversation_id": conversation_id, "native_provider": provider, "status": status})), ctx);
        });
        ctx.observe(&crate::WorkspaceRegistry::handle(ctx), |bridge, _, ctx| {
            bridge.capture(None, ctx)
        });
    }

    pub(super) fn set_instance_id(
        &mut self,
        instance_id: InstanceId,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.instance_id.as_ref() != Some(&instance_id) {
            self.delivery = None;
            self.observation = super::observation::ObservationService::start(instance_id.clone())
                .map_err(|error| {
                    log::warn!(
                        "could not start coordination observations: {}",
                        error.message
                    )
                })
                .ok();
        }
        self.instance_id = Some(instance_id);
        self.capture(None, ctx);
    }

    pub(super) fn stop(&mut self) {
        self.instance_id = None;
        self.delivery = None;
        self.observation = None;
    }

    fn capture(&mut self, signal: Option<serde_json::Value>, ctx: &mut ModelContext<Self>) {
        let Some(instance) = self.instance_id.as_ref() else {
            return;
        };
        if self.observation.is_none() {
            return;
        }
        for window in ctx.window_ids().collect::<Vec<_>>() {
            let Some(parent) = ctx
                .root_view::<crate::root_view::RootView>(window)
                .and_then(|root| root.as_ref(ctx).project_window())
            else {
                continue;
            };
            if self.observed_views.insert(parent.id()) {
                ctx.subscribe_to_view(&parent, |bridge, _, _, ctx| bridge.capture(None, ctx));
            }
            let workspaces = parent
                .as_ref(ctx)
                .projects()
                .map(|(_, workspace)| workspace.clone())
                .collect::<Vec<_>>();
            for workspace in workspaces {
                if self.observed_views.insert(workspace.id()) {
                    ctx.subscribe_to_view(&workspace, |bridge, _, _, ctx| {
                        bridge.capture(None, ctx)
                    });
                }
            }
        }
        // WarpUI retains subscriptions across restorable window closes; do not subscribe twice.
        if let Ok(snapshot) = super::observation::capture(instance, signal, ctx) {
            self.observation.as_ref().unwrap().observe(snapshot);
        }
    }

    pub(super) fn handle_request(
        &mut self,
        request: RequestEnvelope,
        grant: CredentialGrant,
        ctx: &mut ModelContext<Self>,
    ) -> BridgeReply {
        if let Err(error) = ensure_feature_enabled() {
            return ResponseEnvelope::error(request.request_id, error).into();
        }
        if let Err(error) = ensure_protocol_version(request.protocol_version) {
            return ResponseEnvelope::error(request.request_id, error).into();
        }
        let Some(instance_id) = &self.instance_id else {
            return ResponseEnvelope::error(
                request.request_id,
                ControlError::new(
                    ErrorCode::BridgeUnavailable,
                    "local-control bridge has no active instance identity",
                ),
            )
            .into();
        };
        if let Err(error) = validate_request_authority(instance_id, &request.action, &grant) {
            return ResponseEnvelope::error(request.request_id, error).into();
        }
        if let Err(error) = ensure_action_allowed(request.action.kind, ctx) {
            return ResponseEnvelope::error(request.request_id, error).into();
        }
        if let Err(error) = validate_action_target(request.action.kind, &request.target) {
            return ResponseEnvelope::error(request.request_id, error).into();
        }
        if matches!(
            request.action.kind,
            ActionKind::AgentInbox | ActionKind::AgentInboxAck | ActionKind::AgentEvents
        ) {
            return match self
                .observation
                .as_ref()
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::BridgeUnavailable,
                        "coordination observations are unavailable",
                    )
                })
                .and_then(|service| service.request(&request.action, instance_id, ctx))
            {
                Ok(receiver) => BridgeReply::Delivery {
                    request_id: request.request_id,
                    receiver,
                },
                Err(error) => ResponseEnvelope::error(request.request_id, error).into(),
            };
        }
        if request.action.kind == ActionKind::AgentRead {
            return match request
                .action
                .params_as()
                .and_then(|params| super::agents::read_plan(instance_id, params, ctx))
            {
                Ok(plan) => BridgeReply::Read {
                    request_id: request.request_id,
                    plan,
                },
                Err(error) => ResponseEnvelope::error(request.request_id, error).into(),
            };
        }
        if matches!(
            request.action.kind,
            ActionKind::AgentSend
                | ActionKind::AgentMessageInspect
                | ActionKind::AgentMessageCancel
                | ActionKind::AgentMessageList
        ) {
            if self.delivery.is_none() {
                match super::delivery::DeliveryService::start(instance_id.clone(), ctx.spawner()) {
                    Ok(service) => self.delivery = Some(service),
                    Err(error) => return ResponseEnvelope::error(request.request_id, error).into(),
                }
            }
            return match self.delivery.as_ref().unwrap().request(&request.action) {
                Ok(receiver) => BridgeReply::Delivery {
                    request_id: request.request_id,
                    receiver,
                },
                Err(error) => ResponseEnvelope::error(request.request_id, error).into(),
            };
        }
        let result = match request.action.kind {
            ActionKind::PaneRead => request
                .action
                .params_as()
                .and_then(|params| super::agents::read_pane(instance_id, params, ctx)),
            ActionKind::WorkspaceTree | ActionKind::ProjectList | ActionKind::AgentList => {
                request.action.params_as().and_then(|scope| {
                    super::agents::list(request.action.kind, instance_id, scope, ctx)
                })
            }
            ActionKind::ProjectTaskList
            | ActionKind::ProjectTaskCreate
            | ActionKind::ProjectTaskUpdate
            | ActionKind::ProjectTaskComplete
            | ActionKind::ProjectTaskDelete
            | ActionKind::ProjectInspect
            | ActionKind::ProjectCreate
            | ActionKind::ProjectActivate
            | ActionKind::ProjectClose
            | ActionKind::ProjectExport
            | ActionKind::ProjectRestore
            | ActionKind::TabTransfer => super::handlers::projects::handle(
                instance_id,
                &request.action,
                &request.target,
                ctx,
            ),
            ActionKind::AgentLaunch => request
                .action
                .params_as()
                .and_then(|params| super::agents::launch(instance_id, params, ctx)),
            ActionKind::AgentInterrupt => request
                .action
                .params_as()
                .and_then(|params| super::agents::interrupt(instance_id, params, ctx)),
            ActionKind::TabPin | ActionKind::TabUnpin => {
                sections::pin_tab(request.action.kind, &request.target, ctx)
            }
            ActionKind::AgentInspect => request
                .action
                .params_as::<::local_control::agents::AgentTargetParams>()
                .and_then(|params| {
                    super::agents::resolve(instance_id, &params.agent_id, ctx)
                        .map(|entry| entry.data)
                }),
            ActionKind::AgentSend
            | ActionKind::AgentMessageInspect
            | ActionKind::AgentMessageCancel
            | ActionKind::AgentMessageList => unreachable!("delivery handled asynchronously above"),
            ActionKind::AgentRead => unreachable!("read handled asynchronously above"),
            ActionKind::AgentInbox | ActionKind::AgentInboxAck | ActionKind::AgentEvents => {
                unreachable!("observation handled asynchronously above")
            }
            ActionKind::InstanceList => metadata::instance(&self.instance_id),
            ActionKind::InstanceInspect => metadata::inspect(&self.instance_id, ctx),
            ActionKind::AppPing => metadata::ping(&self.instance_id),
            ActionKind::AppVersion => metadata::version(&self.instance_id),
            ActionKind::AppActive => metadata::active(&self.instance_id, ctx),
            ActionKind::CapabilityList => Ok(metadata::capability_list()),
            ActionKind::CapabilityInspect => metadata::capability_inspect(&request.action),
            ActionKind::ActionList => Ok(metadata::action_list()),
            ActionKind::ActionInspect => metadata::action_inspect(&request.action),
            ActionKind::SurfaceList => metadata::surface_list(ctx),
            ActionKind::WindowList => metadata::window_list(&request.target, ctx),
            ActionKind::WindowInspect => metadata::window_inspect(&request.target, ctx),
            ActionKind::TabList => metadata::tab_list(&request.target, ctx),
            ActionKind::TabInspect => metadata::tab_inspect(&request.target, ctx),
            ActionKind::TabGrep => tab_grep::handle(&request.action, &request.target, ctx),
            ActionKind::AppFocus
            | ActionKind::WindowCreate
            | ActionKind::WindowFocus
            | ActionKind::TabCreate
            | ActionKind::TabActivate
            | ActionKind::TabMove
            | ActionKind::PaneSplit
            | ActionKind::PaneFocus
            | ActionKind::PaneNavigate
            | ActionKind::PaneResize
            | ActionKind::PaneMaximize
            | ActionKind::PaneUnmaximize
            | ActionKind::SessionActivate
            | ActionKind::SessionPrevious
            | ActionKind::SessionNext
            | ActionKind::SessionReopenClosed
            | ActionKind::InputInsert
            | ActionKind::InputReplace
            | ActionKind::SurfaceSettingsOpen
            | ActionKind::SurfaceCommandPaletteOpen
            | ActionKind::SurfaceCommandSearchOpen
            | ActionKind::SurfaceThemePickerOpen
            | ActionKind::SurfaceKeybindingsOpen
            | ActionKind::SurfaceWarpDriveOpen
            | ActionKind::SurfaceWarpDriveToggle
            | ActionKind::SurfaceResourceCenterToggle
            | ActionKind::SurfaceAiAssistantToggle
            | ActionKind::SurfaceCodeReviewOpen
            | ActionKind::SurfaceCodeReviewToggle
            | ActionKind::SurfaceProjectExplorerOpen
            | ActionKind::SurfaceGlobalSearchOpen
            | ActionKind::SurfaceConversationListOpen
            | ActionKind::SurfaceLeftPanelToggle
            | ActionKind::SurfaceRightPanelToggle
            | ActionKind::SurfaceVerticalTabsOpen
            | ActionKind::SurfaceVerticalTabsToggle
            | ActionKind::SurfaceAgentManagementOpen
            | ActionKind::FileOpen => app_state::handle(
                &self.instance_id,
                request.action.kind,
                &request.action.params,
                &request.target,
                request.origin_terminal_session_uuid.as_ref(),
                ctx,
            ),
            ActionKind::TabRename => metadata_config::tab_rename(
                &self.instance_id,
                &request.target,
                &request.action,
                ctx,
            ),
            ActionKind::TabResetName => {
                metadata_config::tab_reset_name(&self.instance_id, &request.target, ctx)
            }
            ActionKind::TabColorSet => metadata_config::tab_color_set(
                &self.instance_id,
                &request.target,
                &request.action,
                ctx,
            ),
            ActionKind::TabColorClear => {
                metadata_config::tab_color_clear(&self.instance_id, &request.target, ctx)
            }
            ActionKind::PaneList => metadata::pane_list(&request.target, ctx),
            ActionKind::PaneInspect => metadata::pane_inspect(&request.target, ctx),
            ActionKind::PaneRename => metadata_config::pane_rename(
                &self.instance_id,
                &request.target,
                &request.action,
                ctx,
            ),
            ActionKind::PaneResetName => {
                metadata_config::pane_reset_name(&self.instance_id, &request.target, ctx)
            }
            ActionKind::SessionList => metadata::session_list(&request.target, ctx),
            ActionKind::SessionInspect => metadata::session_inspect(&request.target, ctx),
            ActionKind::ThemeList => settings_surfaces::theme_list(ctx),
            ActionKind::ThemeGet => settings_surfaces::theme_get(ctx),
            ActionKind::ThemeSet
            | ActionKind::ThemeSystemSet
            | ActionKind::ThemeLightSet
            | ActionKind::ThemeDarkSet => metadata_config::theme_set(
                &self.instance_id,
                request.action.kind,
                &request.action,
                ctx,
            ),
            ActionKind::AppearanceGet => settings_surfaces::appearance_get(ctx),
            ActionKind::AppearanceFontSizeIncrease
            | ActionKind::AppearanceFontSizeDecrease
            | ActionKind::AppearanceFontSizeReset
            | ActionKind::AppearanceZoomIncrease
            | ActionKind::AppearanceZoomDecrease
            | ActionKind::AppearanceZoomReset => {
                metadata_config::appearance_mutation(&self.instance_id, request.action.kind, ctx)
            }
            ActionKind::SettingList => settings_surfaces::setting_list(&request.action, ctx),
            ActionKind::SettingGet => settings_surfaces::setting_get(&request.action, ctx),
            ActionKind::SettingSet => metadata_config::setting_set(&request.action, ctx),
            ActionKind::SettingToggle => metadata_config::setting_toggle(&request.action, ctx),
            ActionKind::ToolbeltList
            | ActionKind::ToolbeltButtonCreate
            | ActionKind::ToolbeltButtonDelete
            | ActionKind::ToolbeltButtonMove
            | ActionKind::ToolbeltSuggestionList
            | ActionKind::ToolbeltSuggestionResolve => toolbelt::handle(&request.action, ctx),
            ActionKind::SectionPin
            | ActionKind::SectionUnpin
            | ActionKind::SectionList
            | ActionKind::SectionCreate
            | ActionKind::SectionUpdate
            | ActionKind::SectionDelete
            | ActionKind::SectionMove
            | ActionKind::SectionTabAdd
            | ActionKind::SectionTabRemove => {
                sections::handle(&request.action, &request.target, ctx)
            }
            ActionKind::KeybindingList => settings_surfaces::keybinding_list(ctx),
            ActionKind::KeybindingGet => settings_surfaces::keybinding_get(&request.action, ctx),
            ActionKind::WindowClose => close::window_close(&self.instance_id, &request, ctx),
            ActionKind::TabClose => close::tab_close(&self.instance_id, &request, ctx),
            ActionKind::PaneClose => close::pane_close(&self.instance_id, &request, ctx),
        };
        self.capture(None, ctx);
        match result {
            Ok(data) => ResponseEnvelope::ok(request.request_id, data),
            Err(error) => ResponseEnvelope::error(request.request_id, error),
        }
        .into()
    }
}

pub(crate) fn validate_request_authority(
    instance_id: &InstanceId,
    action: &Action,
    grant: &CredentialGrant,
) -> Result<(), ControlError> {
    grant.verify_for_action(instance_id, action.kind)?;
    if !action.kind.is_implemented() {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!(
                "{} is not implemented by this local-control bridge",
                action.kind.as_str()
            ),
        ));
    }
    validate_action_params(action)
}

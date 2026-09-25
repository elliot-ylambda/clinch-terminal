//! App-wide discovery and exact live-agent targeting for the local CLI.
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::hash::{Hash, Hasher};

use ::local_control::agents::{AgentReadParams, AgentScope};
use ::local_control::{ActionKind, ControlError, ErrorCode, InstanceId};
use base64::Engine as _;
use serde_json::{json, Value};
use warpui::{AppContext, SingletonEntity, ViewHandle};

use crate::agent_resume::AgentResumeProvider;
use crate::root_view::RootView;
use crate::terminal::cli_agent_sessions::{CLIAgentSessionStatus, CLIAgentSessionsModel};
use crate::terminal::{CLIAgent, TerminalView};

use super::conversation::ReadPlan;

pub(super) struct AgentEntry {
    pub data: Value,
    pub terminal: ViewHandle<TerminalView>,
    pub provider: AgentResumeProvider,
    pub conversation_id: Option<String>,
    pub transcript_path: Option<String>,
    pub remote: bool,
}

pub(super) struct Snapshot {
    pub tree: Value,
    pub agents: Vec<AgentEntry>,
    pub panes: Vec<(String, Option<ViewHandle<TerminalView>>)>,
}

fn fingerprint(value: &Value) -> String {
    let mut hash = DefaultHasher::new();
    value.to_string().hash(&mut hash);
    format!("{:016x}", hash.finish())
}

fn opaque_id(parts: Value) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(parts.to_string())
}

/// Enumerates root project containers rather than WorkspaceRegistry's active-project lookup.
pub(super) fn snapshot(
    instance: &InstanceId,
    scope: &AgentScope,
    ctx: &AppContext,
) -> Result<Snapshot, ControlError> {
    let mut windows = Vec::new();
    let mut agents = Vec::new();
    let mut all_panes = Vec::new();
    let mut known_projects = HashSet::new();
    let mut known_sections = HashSet::new();
    for window_id in ctx.window_ids() {
        let Some(root) = ctx.root_view::<RootView>(window_id) else {
            continue;
        };
        let Some(project_window) = root.as_ref(ctx).project_window() else {
            continue;
        };
        let project_window = project_window.as_ref(ctx);
        let mut projects = Vec::new();
        for (project_index, (project_id, workspace)) in project_window.projects().enumerate() {
            let project_id = project_id.opaque_id();
            known_projects.insert(project_id.clone());
            let workspace = workspace.as_ref(ctx);
            for id in workspace.tab_groups.keys() {
                known_sections.insert(id.0.to_string());
            }
            if !scope.projects.is_empty() && !scope.projects.contains(&project_id) {
                continue;
            }
            let project_name = workspace.project_display_name(ctx);
            let mut tabs = Vec::new();
            let mut sections = Vec::new();
            let mut seen_sections = HashSet::new();
            for (tab_index, group_handle) in workspace.tab_views().enumerate() {
                let group = group_handle.as_ref(ctx);
                let tab_id = group_handle.id().to_string();
                let tab_title = group.display_title(ctx);
                let section = workspace
                    .tabs
                    .get(tab_index)
                    .and_then(|tab| tab.group_id)
                    .and_then(|id| {
                        workspace
                            .tab_groups
                            .get(&id)
                            .map(|section| (id.0.to_string(), section))
                    });
                let section_id = section.as_ref().map(|(id, _)| id.clone());
                if !scope.sections.is_empty()
                    && !section_id
                        .as_ref()
                        .is_some_and(|id| scope.sections.contains(id))
                {
                    continue;
                }
                if let Some((id, section)) = &section {
                    if seen_sections.insert(id.clone()) {
                        sections.push(json!({"section_id": id, "name": section.name, "collapsed": section.collapsed, "pinned": section.pinned, "color": format!("{:?}", section.color)}));
                    }
                }
                let mut panes = Vec::new();
                // Include hidden/background panes as well as the ordered visible split tree.
                let mut pane_ids = group.visible_pane_ids();
                let visible: HashSet<_> = pane_ids.iter().copied().collect();
                let mut hidden: Vec<_> = group
                    .pane_ids()
                    .filter(|id| !visible.contains(id))
                    .collect();
                hidden.sort_by_key(|id| format!("{id:?}"));
                pane_ids.extend(hidden);
                for pane_id in pane_ids {
                    let pane_id_string = opaque_id(json!([
                        instance.0,
                        project_id,
                        tab_id,
                        format!("{pane_id:?}")
                    ]));
                    let terminal = group.terminal_view_from_pane_id(pane_id, ctx);
                    all_panes.push((pane_id_string.clone(), terminal.clone()));
                    let mut pane = json!({"pane_id": pane_id_string, "title": group.pane_title(pane_id, ctx), "visible": visible.contains(&pane_id), "active": group.focused_pane_id(ctx) == pane_id, "kind": "other", "content_read": false});
                    if let Some(terminal) = terminal {
                        let terminal_ref = terminal.as_ref(ctx);
                        pane["kind"] = json!("terminal");
                        pane["content_read"] = json!(true);
                        pane["cwd"] = json!(terminal_ref
                            .pwd_if_local(ctx)
                            .or_else(|| terminal_ref.pwd()));
                        if let Some(session) =
                            CLIAgentSessionsModel::as_ref(ctx).session(terminal.id())
                        {
                            let provider = match session.agent {
                                CLIAgent::Claude => {
                                    Some(("claude-code", AgentResumeProvider::Claude))
                                }
                                CLIAgent::Codex => Some(("codex", AgentResumeProvider::Codex)),
                                _ => None,
                            };
                            if let Some((provider_name, provider)) = provider {
                                let conversation_id = session.session_context.session_id.clone();
                                let agent_id = opaque_id(json!([
                                    instance.0,
                                    project_id,
                                    tab_id,
                                    pane_id_string,
                                    provider_name,
                                    conversation_id
                                ]));
                                let state = if !session.supports_rich_status() {
                                    "unknown"
                                } else if session.session_context.stop_reason.is_some() {
                                    "rate_limited"
                                } else {
                                    match session.status {
                                        CLIAgentSessionStatus::Blocked { .. } => "needs_attention",
                                        CLIAgentSessionStatus::Success => "turn_complete",
                                        CLIAgentSessionStatus::InProgress
                                            if session.is_actively_working() =>
                                        {
                                            "working"
                                        }
                                        CLIAgentSessionStatus::InProgress => "idle",
                                    }
                                };
                                let (revision, unavailable) =
                                    terminal_ref.local_control_agent_readiness(ctx);
                                let data = json!({
                                    "agent_id": agent_id, "provider": provider_name, "conversation_id": conversation_id,
                                    "window_id": window_id.to_string(), "project_id": project_id, "project_name": project_name,
                                    "section_id": section_id, "section_name": section.as_ref().and_then(|(_, section)| section.name.clone()),
                                    "tab_id": tab_id, "title": tab_title, "pane_id": pane_id_string, "cwd": pane["cwd"],
                                    "state": state, "state_source": if session.supports_rich_status() { "provider_notifications" } else { "command_detection" },
                                    "input_revision": revision, "ready": unavailable.is_none(), "unavailable_reason": unavailable,
                                    "latest_prompt": session.latest_user_prompt_for_chrome(), "latest_response_preview": session.session_context.response,
                                    "tool_name": session.session_context.tool_name, "tool_input_preview": session.session_context.tool_input_preview,
                                    "capabilities": {"read": true, "send": unavailable.is_none(), "queue": true, "durable_receipts": true, "event_replay": false},
                                });
                                pane["kind"] = json!(provider_name);
                                pane["agent_id"] = json!(agent_id);
                                agents.push(AgentEntry {
                                    data,
                                    terminal: terminal.clone(),
                                    provider,
                                    conversation_id,
                                    transcript_path: session
                                        .session_context
                                        .transcript_path
                                        .clone(),
                                    remote: session.is_remote(),
                                });
                            }
                        }
                    }
                    panes.push(pane);
                }
                tabs.push(json!({"tab_id": tab_id, "name": tab_title, "position": tab_index, "active": workspace.active_tab_index() == tab_index, "section_id": section_id, "panes": panes}));
            }
            projects.push(json!({"project_id": project_id, "name": project_name, "position": project_index, "active": project_window.active_project_index() == project_index, "sections": sections, "tabs": tabs, "tasks": workspace.tasks}));
        }
        windows.push(json!({"window_id": window_id.to_string(), "projects": projects}));
    }
    if scope.projects.iter().any(|id| !known_projects.contains(id))
        || scope.sections.iter().any(|id| !known_sections.contains(id))
    {
        return Err(ControlError::new(
            ErrorCode::StaleTarget,
            "scope contains an unknown project or section ID; discover current IDs with workspace tree",
        ));
    }
    Ok(Snapshot {
        tree: json!({"instance_id": instance.0, "identity_lifetime": "app_instance", "windows": windows, "coverage": "all_open_projects"}),
        agents,
        panes: all_panes,
    })
}

pub(super) fn list(
    kind: ActionKind,
    instance: &InstanceId,
    scope: AgentScope,
    ctx: &AppContext,
) -> Result<Value, ControlError> {
    let snapshot = snapshot(instance, &scope, ctx)?;
    let mut data = match kind {
        ActionKind::WorkspaceTree => snapshot.tree,
        ActionKind::ProjectList => {
            json!({"instance_id": instance.0, "projects": snapshot.tree["windows"].as_array().into_iter().flatten().flat_map(|window| window["projects"].as_array().into_iter().flatten().cloned().map(|mut project| { project["window_id"] = window["window_id"].clone(); project.as_object_mut().unwrap().remove("tabs"); project })).collect::<Vec<_>>() })
        }
        _ => {
            json!({"instance_id": instance.0, "agents": snapshot.agents.into_iter().map(|agent| agent.data).collect::<Vec<_>>() })
        }
    };
    data["action"] = json!(kind.as_str());
    data["snapshot_cursor"] = json!(format!("{}:{}", instance.0, fingerprint(&data)));
    data["observed_at"] = json!(chrono::Utc::now().to_rfc3339());
    Ok(data)
}

pub(super) fn resolve(
    instance: &InstanceId,
    agent_id: &str,
    ctx: &AppContext,
) -> Result<AgentEntry, ControlError> {
    snapshot(instance, &AgentScope::default(), ctx)?
        .agents
        .into_iter()
        .find(|entry| entry.data["agent_id"].as_str() == Some(agent_id))
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::StaleTarget,
                "agent identity is no longer present in this app instance; run agent list again",
            )
        })
}

pub(super) fn read_plan(
    instance: &InstanceId,
    params: AgentReadParams,
    ctx: &AppContext,
) -> Result<ReadPlan, ControlError> {
    if params.limit == 0 || params.limit > 500 || (params.tail && params.after.is_some()) {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "read limit must be 1–500; --tail and --after are mutually exclusive",
        ));
    }
    let entry = resolve(instance, &params.agent_id, ctx)?;
    Ok(ReadPlan {
        params,
        agent: entry.data,
        provider: entry.provider,
        session_id: entry.conversation_id,
        transcript_path: entry.transcript_path,
        remote: entry.remote,
    })
}

pub(super) fn read_pane(
    instance: &InstanceId,
    params: ::local_control::agents::PaneReadParams,
    ctx: &AppContext,
) -> Result<Value, ControlError> {
    if params.max_bytes == 0 || params.max_bytes > ::local_control::agents::MAX_READ_BYTES {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "max_bytes must be between 1 and 262144",
        ));
    }
    let terminal = snapshot(instance, &AgentScope::default(), ctx)?
        .panes
        .into_iter()
        .find(|(id, _)| *id == params.pane_id)
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::StaleTarget,
                "pane is no longer present; discover it with workspace tree",
            )
        })?
        .1
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::UnsupportedAction,
                "this pane is not a readable terminal",
            )
        })?;
    let bytes = terminal
        .as_ref(ctx)
        .remote_control_scrollback_bytes(params.max_bytes + 1);
    let truncated = bytes.len() > params.max_bytes;
    let slice = &bytes[bytes.len().saturating_sub(params.max_bytes)..];
    Ok(
        json!({"action": "pane.read", "pane_id": params.pane_id, "format": "ansi", "source": "bounded_terminal_snapshot", "coverage": "partial", "content_truncated": truncated, "text": String::from_utf8_lossy(slice)}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root_view::NewWorkspaceSource;
    use crate::terminal::cli_agent_sessions::{
        CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext,
    };
    use crate::GlobalResourceHandles;
    use warp_core::channel::{Channel, ChannelConfig, ChannelState};
    use warp_core::AppId;
    use warpui::platform::WindowStyle;
    use warpui::App;

    #[test]
    fn discovers_inactive_projects_and_rejects_replaced_conversations() {
        App::test((), |mut app| async move {
            crate::workspace::view::tests::initialize_app(&mut app);
            app.update(crate::root_view::init);
            ChannelState::set(ChannelState::new(
                Channel::Local,
                ChannelConfig::no_backend(AppId::new("test", "warp", "WarpTest"), "warp-test.log"),
            ));
            let resources = GlobalResourceHandles::mock(&mut app);
            let (_, root) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
                RootView::new(
                    resources,
                    NewWorkspaceSource::Empty {
                        previous_active_window: None,
                        shell: None,
                    },
                    ctx,
                )
            });
            let project_window = root.read(&app, |root, _| root.project_window()).unwrap();
            project_window.update(&mut app, |window, ctx| window.add_project(ctx));
            let terminals = project_window.read(&app, |window, ctx| {
                window
                    .projects()
                    .map(|(_, workspace)| {
                        workspace
                            .as_ref(ctx)
                            .active_tab_pane_group()
                            .as_ref(ctx)
                            .terminal_views(ctx)[0]
                            .clone()
                    })
                    .collect::<Vec<_>>()
            });
            for (index, terminal) in terminals.iter().enumerate() {
                app.update(|ctx| {
                    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                        sessions.set_session(
                            terminal.id(),
                            CLIAgentSession {
                                agent: if index == 0 {
                                    CLIAgent::Claude
                                } else {
                                    CLIAgent::Codex
                                },
                                status: CLIAgentSessionStatus::Success,
                                session_context: CLIAgentSessionContext {
                                    session_id: Some(format!("conversation-{index}")),
                                    ..Default::default()
                                },
                                input_state: CLIAgentInputState::Closed,
                                should_auto_toggle_input: false,
                                listener: None,
                                plugin_version: None,
                                remote_host: None,
                                draft_text: None,
                                custom_command_prefix: None,
                                received_rich_notification: true,
                                has_observed_turn_activity: true,
                                turn_interrupted_by_user: false,
                                prompt_history: Default::default(),
                                prompt_history_load_state: Default::default(),
                                prompt_history_generation: 0,
                            },
                            ctx,
                        )
                    })
                });
            }
            let instance = InstanceId("test-instance".into());
            let before = project_window.read(&app, |window, _| window.active_project_index());
            let snapshot =
                app.read(|ctx| snapshot(&instance, &AgentScope::default(), ctx).unwrap());
            assert_eq!(snapshot.agents.len(), 2);
            assert_eq!(
                snapshot.tree["windows"][0]["projects"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            let first = snapshot.agents[0].data["agent_id"]
                .as_str()
                .unwrap()
                .to_owned();
            let project_id = snapshot.agents[0].data["project_id"]
                .as_str()
                .unwrap()
                .to_owned();
            app.read(|ctx| {
                assert!(resolve(&instance, &first, ctx).is_ok());
                assert!(resolve(&InstanceId("other-instance".into()), &first, ctx).is_err());
                let filtered = super::snapshot(
                    &instance,
                    &AgentScope {
                        projects: vec![project_id],
                        sections: vec![],
                    },
                    ctx,
                )
                .unwrap();
                assert_eq!(filtered.agents.len(), 1);
                assert!(super::snapshot(
                    &instance,
                    &AgentScope {
                        projects: vec!["missing".into()],
                        sections: vec![]
                    },
                    ctx
                )
                .is_err());
            });
            assert_eq!(
                before,
                project_window.read(&app, |window, _| window.active_project_index())
            );
            app.update(|ctx| {
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                    let mut replacement = sessions.session(terminals[0].id()).unwrap().clone();
                    replacement.session_context.session_id =
                        Some("replacement-conversation".into());
                    sessions.set_session(terminals[0].id(), replacement, ctx);
                })
            });
            app.read(|ctx| assert!(resolve(&instance, &first, ctx).is_err()));
        });
    }
}

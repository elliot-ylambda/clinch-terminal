use std::rc::Rc;

use warp_core::settings::Setting as _;
use warpui::{App, AppContext, SingletonEntity, ViewContext};

use super::super::{AIBlockMetadata, RichContentMetadata, RichContentType};
use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentInput, ServerOutputId, UserQueryMode};
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::block::cli_controller::UserTakeOverReason;
use crate::ai::blocklist::model::{
    AIBlockModel, AIBlockOutputStatus, AIRequestType, OutputStatusUpdateCallback,
};
use crate::ai::blocklist::{AIBlock, ClientIdentifiers};
use crate::ai::llms::LLMId;
use crate::features::FeatureFlag;
use crate::settings::AISettings;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};
use crate::terminal::model::ansi::{self, BootstrappedValue, Handler as _, InitShellValue};
use crate::terminal::session_settings::SessionSettings;
use crate::terminal::shared_session::SharedSessionSource;
use crate::terminal::CLIAgent;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

#[test]
#[cfg(feature = "local_tty")]
fn claude_transfer_launches_codex_with_session_transcript_hint() {
    let transfer = build_cli_agent_transfer(
        CLIAgent::Claude,
        &CLIAgentSessionContext {
            session_id: Some("claude-session-123".to_string()),
            cwd: Some("/tmp/project".to_string()),
            ..Default::default()
        },
        ShellType::Zsh,
    )
    .expect("Claude session should be transferable");

    assert_eq!(transfer.source_agent, CLIAgent::Claude);
    assert!(transfer
        .launch_command
        .starts_with("codex --dangerously-bypass-approvals-and-sandbox "));
    assert!(transfer.launch_command.contains("last Claude Code agent"));
    assert!(transfer.launch_command.contains("claude-session-123.jsonl"));
    assert!(transfer.launch_command.contains("/tmp/project"));
    assert!(!transfer.launch_command.contains("/export"));
}

#[test]
#[cfg(feature = "local_tty")]
fn codex_transfer_launches_claude_with_exact_transcript_path() {
    let transfer = build_cli_agent_transfer(
        CLIAgent::Codex,
        &CLIAgentSessionContext {
            session_id: Some("codex-session-456".to_string()),
            transcript_path: Some("/tmp/o'malley/rollout-codex-session-456.jsonl".to_string()),
            ..Default::default()
        },
        ShellType::Zsh,
    )
    .expect("Codex session should be transferable");

    assert_eq!(transfer.source_agent, CLIAgent::Codex);
    assert!(transfer
        .launch_command
        .starts_with("claude --dangerously-skip-permissions "));
    assert!(transfer.launch_command.contains("last Codex agent"));
    assert!(transfer
        .launch_command
        .contains("rollout-codex-session-456.jsonl"));
    assert!(
        transfer.launch_command.contains("'\"'\"'"),
        "transcript path must remain shell-quoted: {}",
        transfer.launch_command
    );
}

#[test]
#[cfg(feature = "local_tty")]
fn transfer_requires_supported_agent_and_conversation_reference() {
    assert!(build_cli_agent_transfer(
        CLIAgent::Claude,
        &CLIAgentSessionContext::default(),
        ShellType::Zsh,
    )
    .is_none());
    assert!(build_cli_agent_transfer(
        CLIAgent::Gemini,
        &CLIAgentSessionContext {
            session_id: Some("session-123".to_string()),
            ..Default::default()
        },
        ShellType::Zsh,
    )
    .is_none());
}

struct PendingAIBlockModel {
    conversation_id: AIConversationId,
    input: Vec<AIAgentInput>,
    model_id: LLMId,
}

impl PendingAIBlockModel {
    fn new(conversation_id: AIConversationId, input: Vec<AIAgentInput>) -> Self {
        Self {
            conversation_id,
            input,
            model_id: LLMId::from("fake-llm"),
        }
    }
}

impl AIBlockModel for PendingAIBlockModel {
    type View = AIBlock;

    fn status(&self, _app: &AppContext) -> AIBlockOutputStatus {
        AIBlockOutputStatus::Pending
    }

    fn server_output_id(&self, _app: &AppContext) -> Option<ServerOutputId> {
        None
    }

    fn model_id(&self, _app: &AppContext) -> Option<LLMId> {
        None
    }

    fn base_model<'a>(&'a self, _app: &'a AppContext) -> Option<&'a LLMId> {
        Some(&self.model_id)
    }

    fn inputs_to_render<'a>(&'a self, _app: &'a AppContext) -> &'a [AIAgentInput] {
        &self.input
    }

    fn conversation_id(&self, _app: &AppContext) -> Option<AIConversationId> {
        Some(self.conversation_id)
    }

    fn on_updated_output(
        &self,
        _callback: OutputStatusUpdateCallback<AIBlock>,
        _ctx: &mut ViewContext<AIBlock>,
    ) {
    }

    fn request_type(&self, _app: &AppContext) -> AIRequestType {
        AIRequestType::Active
    }
}

fn simulate_user_started_long_running_command(view: &mut TerminalView) {
    {
        let mut model = view.model.lock();
        model.init_shell(InitShellValue {
            session_id: 0.into(),
            shell: "zsh".to_owned(),
            ..Default::default()
        });
        model.bootstrapped(BootstrappedValue {
            shell: "zsh".to_owned(),
            ..Default::default()
        });
        model.simulate_long_running_block("ssh localhost", "Password:");
    }
}

#[test]
fn sticky_toolbelt_gate_handles_terminal_visibility_and_cli_precedence() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.read(&app, |view, ctx| {
            let model = view.model.lock();
            assert!(view.should_render_sticky_toolbelt_footer(&model, ctx));
        });

        SessionSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings.show_terminal_footer.set_value(false, ctx).unwrap();
        });
        terminal.read(&app, |view, ctx| {
            let model = view.model.lock();
            assert!(!view.should_render_sticky_toolbelt_footer(&model, ctx));
        });

        SessionSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings.show_terminal_footer.set_value(true, ctx).unwrap();
        });
        terminal.update(&mut app, |view, _| {
            view.model.lock().set_mode(ansi::Mode::SwapScreen {
                save_cursor_and_clear_screen: true,
            });
        });
        terminal.read(&app, |view, ctx| {
            let model = view.model.lock();
            assert!(!view.should_render_sticky_toolbelt_footer(&model, ctx));
        });

        SessionSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings.show_terminal_footer.set_value(false, ctx).unwrap();
        });
        CLIAgentSessionsModel::handle(&app).update(&mut app, |sessions, ctx| {
            sessions.set_session(
                terminal.id(),
                CLIAgentSession {
                    agent: CLIAgent::Claude,
                    status: CLIAgentSessionStatus::InProgress,
                    session_context: CLIAgentSessionContext::default(),
                    input_state: CLIAgentInputState::Closed,
                    listener: None,
                    plugin_version: None,
                    remote_host: None,
                    draft_text: None,
                    custom_command_prefix: None,
                    received_rich_notification: false,
                    has_observed_turn_activity: false,
                    turn_interrupted_by_user: false,
                    prompt_history: Default::default(),
                    prompt_history_load_state: Default::default(),
                    prompt_history_generation: 0,
                    should_auto_toggle_input: false,
                },
                ctx,
            );
        });
        terminal.read(&app, |view, ctx| {
            let model = view.model.lock();
            assert!(view.should_render_sticky_toolbelt_footer(&model, ctx));
            assert_eq!(
                view.use_agent_footer.as_ref(ctx).cli_agent(ctx),
                Some(CLIAgent::Claude)
            );
        });
    });
}

#[test]
fn manual_resume_wrapper_exposes_history_identity_from_the_active_command() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.init_shell(InitShellValue {
                session_id: 0.into(),
                shell: "zsh".to_owned(),
                ..Default::default()
            });
            model.bootstrapped(BootstrappedValue {
                shell: "zsh".to_owned(),
                ..Default::default()
            });
            model.simulate_long_running_block(
                "clinch_agent_resume_launch claude requested-session --dangerously-skip-permissions",
                "",
            );
            drop(model);

            assert_eq!(
                view.active_resume_command_seed(),
                Some((
                    AgentResumeProvider::Claude,
                    "requested-session".to_owned()
                ))
            );
        });
    });
}

fn transition_to_user_handoff_state(
    view: &mut TerminalView,
    reason: UserTakeOverReason,
    ctx: &mut ViewContext<TerminalView>,
) -> AIConversationId {
    let conversation_id = view.agent_view_controller().update(ctx, |controller, ctx| {
        controller
            .try_enter_inline_agent_view(None, AgentViewEntryOrigin::LongRunningCommand, ctx)
            .expect("inline agent view should create a conversation")
    });
    view.model
        .lock()
        .block_list_mut()
        .active_block_mut()
        .set_is_agent_tagged_in(true);

    let task_id = TaskId::new("test-task".to_owned());
    view.model
        .lock()
        .block_list_mut()
        .active_block_mut()
        .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
        .expect("tagged-in command should transition to agent-monitored");

    view.cli_subagent_controller.update(ctx, |controller, ctx| {
        controller.switch_control_to_user(reason, ctx);
    });

    conversation_id
}

fn insert_pending_ai_block(
    view: &mut TerminalView,
    conversation_id: AIConversationId,
    ctx: &mut ViewContext<TerminalView>,
) {
    let ai_block_model = Rc::new(PendingAIBlockModel::new(
        conversation_id,
        vec![AIAgentInput::UserQuery {
            query: "help with this running command".to_owned(),
            context: vec![].into(),
            static_query_type: None,
            referenced_attachments: Default::default(),
            user_query_mode: UserQueryMode::default(),
            running_command: None,
            intended_agent: None,
        }],
    ));
    let ai_block = ctx.add_typed_action_view(|ctx| {
        AIBlock::new(
            ai_block_model.clone(),
            view.model.clone(),
            ClientIdentifiers {
                client_exchange_id: Default::default(),
                conversation_id,
                response_stream_id: None,
            },
            view.ai_controller.clone(),
            view.get_relevant_files_controller.clone(),
            None,
            None,
            view.ai_action_model.clone(),
            view.ai_context_model.clone(),
            view.find_model.clone(),
            view.active_session.clone(),
            &view.cli_subagent_controller,
            &view.model_events_handle,
            view.agent_view_controller.clone(),
            view.ambient_agent_view_model.clone(),
            view.view_handle.clone(),
            view.id(),
            ctx,
        )
    });

    view.insert_rich_content(
        Some(RichContentType::AIBlock),
        ai_block.clone(),
        Some(RichContentMetadata::AIBlock(AIBlockMetadata {
            exchange_id: Default::default(),
            conversation_id,
            ai_block_handle: ai_block,
        })),
        RichContentInsertionPosition::Append {
            insert_below_long_running_block: false,
        },
        ctx,
    );
}

#[test]
fn use_agent_footer_renders_for_manual_handoff_even_when_user_command_footer_setting_disabled() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            let _ = settings
                .should_render_use_agent_footer_for_user_commands
                .set_value(false, ctx);
        });

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            simulate_user_started_long_running_command(view);

            view.maybe_show_use_agent_footer_in_blocklist(ctx);
            {
                let model = view.model.lock();
                assert!(!view.should_render_use_agent_footer(&model, ctx));
                let active_block_index = model.block_list().active_block_index();
                assert!(model
                    .block_list()
                    .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                    .is_none());
            }

            transition_to_user_handoff_state(view, UserTakeOverReason::Manual, ctx);

            view.maybe_show_use_agent_footer_in_blocklist(ctx);
            let model = view.model.lock();
            assert!(view.should_render_use_agent_footer(&model, ctx));
            let active_block_index = model.block_list().active_block_index();
            let rendered_footer_view_id = model
                .block_list()
                .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                .map(|(_, item)| item.view_id);
            assert_eq!(rendered_footer_view_id, Some(view.use_agent_footer.id()));
        });
    })
}

#[test]
fn use_agent_footer_renders_for_manual_handoff_when_unfinished_ai_block_remains() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        FeatureFlag::AgentView.set_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            simulate_user_started_long_running_command(view);

            let conversation_id = view.agent_view_controller().update(ctx, |controller, ctx| {
                controller
                    .try_enter_inline_agent_view(
                        None,
                        AgentViewEntryOrigin::LongRunningCommand,
                        ctx,
                    )
                    .expect("inline agent view should create a conversation")
            });
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_is_agent_tagged_in(true);
            let task_id = TaskId::new("test-task".to_owned());
            view.model
                .lock()
                .block_list_mut()
                .active_block_mut()
                .set_agent_interaction_mode_for_agent_monitored_command(&task_id, conversation_id)
                .expect("tagged-in command should transition to agent-monitored");

            insert_pending_ai_block(view, conversation_id, ctx);
            assert!(view.active_ai_block(ctx).is_some());

            view.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.switch_control_to_user(UserTakeOverReason::Manual, ctx);
            });
        });

        terminal.read(&app, |view, ctx| {
            let model = view.model.lock();
            assert!(view.should_render_use_agent_footer(&model, ctx));
            let active_block_index = model.block_list().active_block_index();
            let rendered_footer_view_id = model
                .block_list()
                .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                .map(|(_, item)| item.view_id);
            assert_eq!(rendered_footer_view_id, Some(view.use_agent_footer.id()));
        });
    })
}

/// During the setup phase of a cloud agent (ambient) shared session — LRCs
/// running before any CLI agent has started — the use-agent footer must stay
/// hidden.
#[test]
fn use_agent_footer_hidden_during_cloud_agent_setup_lrc() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            simulate_user_started_long_running_command(view);

            // Cloud agent setup phase: ambient source type set, LRC running,
            // NO CLIAgentSession registered yet.
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::ambient_agent(None));
            assert!(view.model.lock().is_shared_ambient_agent_session());
            assert!(
                CLIAgentSessionsModel::as_ref(ctx)
                    .session(view.id())
                    .is_none(),
                "precondition: no CLI agent session yet",
            );

            view.maybe_show_use_agent_footer_in_blocklist(ctx);

            let model = view.model.lock();
            assert!(
                !view.should_render_use_agent_footer(&model, ctx),
                "footer should be hidden during cloud agent setup LRCs",
            );
            let active_block_index = model.block_list().active_block_index();
            assert!(
                model
                    .block_list()
                    .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                    .is_none(),
                "footer rich content should not be in the blocklist during cloud setup",
            );
        });
    })
}

/// When viewing a shared cloud-agent (ambient agent) session whose sharer is
/// running a CLI agent, the CLI agent footer should remain outside the block list.
#[test]
fn cli_agent_footer_is_sticky_for_viewer_of_shared_cloud_agent_session() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            simulate_user_started_long_running_command(view);

            // Mark the model as a shared ambient (cloud) agent session, mirroring
            // what the viewer's terminal manager does on `JoinedSuccessfully`.
            view.model
                .lock()
                .set_shared_session_source(SharedSessionSource::ambient_agent(None));
            assert!(view.model.lock().is_shared_ambient_agent_session());

            // Inject a CLI agent session as `apply_cli_agent_state_update` would on
            // the viewer when the sharer reports an active CLI agent.
            let view_id = view.id();
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Claude,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        listener: None,
                        plugin_version: None,
                        remote_host: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                        has_observed_turn_activity: false,
                        turn_interrupted_by_user: false,
                        prompt_history: Default::default(),
                        prompt_history_load_state: Default::default(),
                        prompt_history_generation: 0,
                        should_auto_toggle_input: false,
                    },
                    ctx,
                );
            });

            view.maybe_show_use_agent_footer_in_blocklist(ctx);

            let model = view.model.lock();
            assert!(
                view.should_render_use_agent_footer(&model, ctx),
                "footer should render for viewer of shared cloud agent session with CLI agent",
            );
            assert!(
                view.should_render_sticky_toolbelt_footer(&model, ctx),
                "CLI agent footer should render as a persistent terminal-layout child",
            );
            let active_block_index = model.block_list().active_block_index();
            let rendered_footer_view_id = model
                .block_list()
                .last_non_hidden_rich_content_block_after_block(Some(active_block_index))
                .map(|(_, item)| item.view_id);
            assert_eq!(rendered_footer_view_id, None);
        });
    })
}

/// Regression test: a restored pane seeds its CLI agent session before the
/// resume command runs (see `seed_resumed_session`), which suppresses the
/// `Started` event that normally marks the command's block as an agent TUI
/// block. Long-running-command detection must still apply the agent grid
/// behavior (e.g. trailing-blank-row trimming) to the resumed command's block,
/// otherwise the block renders a full screen of blank rows below the TUI.
#[test]
fn long_running_resume_command_trims_trailing_blanks_despite_seeded_session() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _trim_guard = FeatureFlag::TrimTrailingBlankLines.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);

        terminal.update(&mut app, |view, ctx| {
            // Restore seeds the session while a pre-command block is active.
            let view_id = view.view_id;
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, ctx| {
                sessions.set_session(
                    view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Codex,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext {
                            session_id: Some("seeded-session".to_owned()),
                            ..Default::default()
                        },
                        input_state: CLIAgentInputState::Closed,
                        listener: None,
                        plugin_version: None,
                        remote_host: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        received_rich_notification: false,
                        has_observed_turn_activity: false,
                        turn_interrupted_by_user: false,
                        prompt_history: Default::default(),
                        prompt_history_load_state: Default::default(),
                        prompt_history_generation: 0,
                        should_auto_toggle_input: false,
                    },
                    ctx,
                );
            });

            {
                let mut model = view.model.lock();
                model.init_shell(InitShellValue {
                    session_id: 0.into(),
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                model.bootstrapped(BootstrappedValue {
                    shell: "zsh".to_owned(),
                    ..Default::default()
                });
                // The seed-era block completes before the resume command runs.
                model.simulate_cmd("echo warmup");
                model.finish_block();

                // The resumed agent redraws like a TUI: the visible frame
                // occupies the top rows, while an earlier frame touched the
                // bottom of the screen (and was cleared by a later repaint),
                // leaving a trail of blank rows below the content.
                model.simulate_long_running_block(
                    "clinch_agent_resume_launch codex seeded-session --model gpt-5.6-sol",
                    "Hooks need review\r\nPress enter to confirm",
                );
            }

            let content_len = {
                let model = view.model.lock();
                model
                    .block_list()
                    .active_block()
                    .grid_of_type(crate::terminal::GridType::Output)
                    .expect("active block should have an output grid")
                    .len_displayed()
            };

            {
                let mut model = view.model.lock();
                model.process_bytes("\x1b[99;1Hstatus\x1b[2K");
            }

            view.handle_long_running_command_cli_agent_detection(ctx);

            let model = view.model.lock();
            let output_grid = model
                .block_list()
                .active_block()
                .grid_of_type(crate::terminal::GridType::Output)
                .expect("active block should have an output grid");
            assert_eq!(
                output_grid.len_displayed(),
                content_len,
                "agent block should trim trailing blank rows below the visible TUI content"
            );
        });
    })
}

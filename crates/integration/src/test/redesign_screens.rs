use std::collections::HashMap;

use pathfinder_geometry::vector::{vec2f, Vector2F};
use settings::Setting as _;
use warp::integration_testing::autoupdate::{
    set_clinch_update_available, set_clinch_update_step, AutoupdateStage, ClinchDownloadProgress,
    ClinchUpdateFailure,
};
use warp::integration_testing::terminal::util::ExpectedExitStatus;
use warp::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks, execute_command_for_single_terminal_in_tab,
    execute_echo_str, wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::{single_terminal_view_for_tab, workspace_view};
use warp::settings::{AutoCreateWorktreesForNewTabs, Theme};
use warp::themes::theme::ThemeKind;
use warp::workspace::view::auto_coordinate::AUTO_COORDINATE_PILL_POSITION_ID;
use warp::workspace::WorkspaceAction;
use warpui_core::event::{Event, ModifiersState};
use warpui_core::integration::TestStep;
use warpui_core::{App, WindowId};

use crate::Builder;

fn shot(name: &str) -> TestStep {
    TestStep::new(name).with_take_screenshot(format!("{name}.png"))
}

/// Dispatches a workspace action on the active window, then waits a moment for it to render.
fn workspace_action(name: &str, action: WorkspaceAction) -> TestStep {
    TestStep::new(name)
        .with_action(move |app, window_id, _| {
            let workspace = workspace_view(app, window_id);
            app.update(|ctx| {
                ctx.dispatch_typed_action_for_view(window_id, workspace.id(), &action);
            });
        })
        .set_post_step_pause(std::time::Duration::from_millis(600))
}

fn press(name: &str, keystrokes: &[&str]) -> TestStep {
    TestStep::new(name)
        .with_keystrokes(keystrokes)
        .set_post_step_pause(std::time::Duration::from_millis(400))
}

/// Saves native screenshots of the redesigned surfaces for side-by-side comparison with
/// `web/design-prototype`. Runs in an isolated window and profile:
///
/// ```sh
/// WARPUI_USE_REAL_DISPLAY_IN_INTEGRATION_TESTS=1 \
///   cargo run -p integration --bin integration -- test_redesign_screens
/// ```
pub fn test_redesign_screens() -> Builder {
    redesign_screens(HashMap::new())
}

/// The same screens in the light theme.
pub fn test_redesign_screens_paper() -> Builder {
    redesign_screens(HashMap::from([(
        Theme::storage_key().to_string(),
        serde_json::to_string(&ThemeKind::ClinchPaper).expect("theme kind serializes"),
    )]))
}

fn redesign_screens(mut user_defaults: HashMap<String, String>) -> Builder {
    // New tabs would otherwise add real worktrees to the repo the test runs inside.
    user_defaults.insert(
        AutoCreateWorktreesForNewTabs::storage_key().to_string(),
        false.to_string(),
    );
    Builder::new()
        .with_real_display()
        .with_user_defaults(user_defaults)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_echo_str(0, "hello from the redesign"))
        .with_step(press("second tab", &["cmd-t"]))
        .with_step(press("third tab", &["cmd-t"]))
        .with_step(press("back to first tab", &["cmd-1"]))
        .with_step(workspace_action(
            "new section",
            WorkspaceAction::CreateNewTabGroup,
        ))
        .with_step(shot("main"))
        .with_step(press("open palette", &["cmd-p"]))
        .with_step(shot("palette"))
        .with_step(press("close palette", &["escape"]))
        .with_step(press("open find", &["cmd-f"]))
        .with_step(shot("find"))
        .with_step(press("close find", &["escape"]))
        .with_step(press("split right", &["cmd-d"]))
        .with_step(shot("split"))
        .with_step(press("open settings", &["cmd-,"]))
        .with_step(shot("settings"))
}

/// The agent surfaces, against a stand-in `claude` that only waits, so no real agent or account is
/// involved: the row glyph, footer and More menu all key off the command name.
pub fn test_redesign_agent_screens() -> Builder {
    Builder::new()
        .with_real_display()
        .with_user_defaults(HashMap::from([(
            AutoCreateWorktreesForNewTabs::storage_key().to_string(),
            false.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "mkdir -p ~/bin; printf '#!/bin/sh\\nsleep 60\\n' > ~/bin/claude; chmod +x ~/bin/claude; export PATH=~/bin:$PATH"
                .to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            TestStep::new("start the stand-in agent")
                .with_input_string("claude", Some(&["enter"]))
                .set_post_step_pause(std::time::Duration::from_secs(3)),
        )
        .with_step(workspace_action("open coordinator", WorkspaceAction::OpenCoordinatorSession))
        .with_step(
            TestStep::new("let the coordinator start")
                .set_post_step_pause(std::time::Duration::from_secs(3)),
        )
        .with_step(shot("agent"))
        .with_step(workspace_action(
            "turn on auto-coordinate",
            WorkspaceAction::SetAutoCoordinate(true),
        ))
        .with_step(shot("coordinator-auto-on"))
        .with_step(
            TestStep::new("open auto-coordinate settings")
                .with_click_on_saved_position(AUTO_COORDINATE_PILL_POSITION_ID)
                .set_post_step_pause(std::time::Duration::from_millis(400)),
        )
        .with_step(shot("coordinator-auto-settings"))
        .with_step(
            TestStep::new("close auto-coordinate settings")
                .with_click_on_saved_position(AUTO_COORDINATE_PILL_POSITION_ID)
                .set_post_step_pause(std::time::Duration::from_millis(400)),
        )
        .with_step(
            TestStep::new("open More")
                .with_click_on_saved_position("agent_footer:more")
                .set_post_step_pause(std::time::Duration::from_millis(400)),
        )
        .with_step(shot("agent-more"))
        .with_step(
            TestStep::new("close More")
                .with_click_on_saved_position("agent_footer:more")
                .set_post_step_pause(std::time::Duration::from_millis(400)),
        )
        .with_step(workspace_action(
            "open footer editor",
            WorkspaceAction::OpenCLIAgentToolbarEditor,
        ))
        .with_step(shot("footer-editor"))
        // Drag "Continue" (row 5) to the top, then an available button into the footer list.
        .with_step(drag("drag a row to the top", "chip_cfg_u_4", "chip_cfg_u_0", -10.))
        .with_step(shot("footer-editor-reordered"))
        .with_step(drag("drag an available button in", "chip_cfg_n_0", "chip_cfg_u_1", 0.))
        .with_step(shot("footer-editor-added"))
        .with_step(press("close the editor", &["escape"]))
        .with_step(workspace_action(
            "switch the coordinator off",
            WorkspaceAction::SetCoordinatorOn(false),
        ))
        .with_step(shot("coordinator-off"))
        .with_step(workspace_action(
            "switch the coordinator back on",
            WorkspaceAction::SetCoordinatorOn(true),
        ))
        .with_step(
            TestStep::new("let the coordinator start again")
                .set_post_step_pause(std::time::Duration::from_secs(2)),
        )
        .with_step(shot("coordinator-on"))
}

/// The update pop-up at each of its steps, and footer buttons dragged around by real mouse events,
/// in the coordinator's footer and an agent's. The More menu's buttons are dragged out of it.
pub fn test_update_popup_and_footer_drag_screens() -> Builder {
    let update_step = |name: &'static str,
                       stage: fn() -> AutoupdateStage,
                       progress: Option<ClinchDownloadProgress>,
                       failure: Option<ClinchUpdateFailure>| {
        vec![
            TestStep::new(name)
                .with_action(move |app, _, _| {
                    app.update(|ctx| set_clinch_update_step(stage(), progress, failure, ctx));
                })
                .set_post_step_pause(std::time::Duration::from_millis(300)),
            shot(name),
        ]
    };
    let mut builder = Builder::new()
        .with_real_display()
        .with_user_defaults(HashMap::from([(
            AutoCreateWorktreesForNewTabs::storage_key().to_string(),
            false.to_string(),
        )]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "mkdir -p ~/bin; printf '#!/bin/sh\\nsleep 300\\n' > ~/bin/claude; chmod +x ~/bin/claude; export PATH=~/bin:$PATH"
                .to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            TestStep::new("start the stand-in agent")
                .with_input_string("claude", Some(&["enter"]))
                .set_post_step_pause(std::time::Duration::from_secs(3)),
        )
        .with_step(workspace_action(
            "open coordinator",
            WorkspaceAction::OpenCoordinatorSession,
        ))
        .with_step(
            TestStep::new("let the coordinator start")
                .set_post_step_pause(std::time::Duration::from_secs(3)),
        )
        .with_step(shot("coordinator-row"))
        .with_step(workspace_action(
            "new section",
            WorkspaceAction::CreateNewTabGroup,
        ))
        .with_step(press("finish naming the section", &["escape"]))
        .with_step(shot("section-below-coordinator"))
        .with_step(
            TestStep::new("hover the coordinator row")
                .with_hover_over_saved_position(AUTO_COORDINATE_PILL_POSITION_ID)
                .set_post_step_pause(std::time::Duration::from_millis(900)),
        )
        .with_step(shot("coordinator-row-hovered"))
        .with_step(
            TestStep::new("open auto-coordinate settings")
                .with_click_on_saved_position(AUTO_COORDINATE_PILL_POSITION_ID)
                .set_post_step_pause(std::time::Duration::from_millis(900)),
        )
        .with_step(shot("auto-settings-with-row-hovered"))
        .with_step(
            TestStep::new("close auto-coordinate settings")
                .with_click_on_saved_position(AUTO_COORDINATE_PILL_POSITION_ID)
                .set_post_step_pause(std::time::Duration::from_millis(400)),
        )
        .with_step(press("back to the coordinator", &["cmd-1"]))
        .with_step(
            TestStep::new("offer an update")
                .with_action(|app, window_id, _| {
                    app.update(set_clinch_update_available);
                    let workspace = workspace_view(app, window_id);
                    app.update(|ctx| {
                        ctx.dispatch_typed_action_for_view(
                            window_id,
                            workspace.id(),
                            &WorkspaceAction::ApplyUpdate,
                        );
                    });
                })
                .set_post_step_pause(std::time::Duration::from_millis(400)),
        )
        .with_step(shot("update-confirm"));
    for step in update_step(
        "update-downloading",
        || AutoupdateStage::DownloadingUpdate,
        Some(ClinchDownloadProgress::Downloading {
            received: 52_000_000,
            total: 123_400_000,
        }),
        None,
    ) {
        builder = builder.with_step(step);
    }
    for step in update_step(
        "update-verifying",
        || AutoupdateStage::DownloadingUpdate,
        Some(ClinchDownloadProgress::Verifying),
        None,
    ) {
        builder = builder.with_step(step);
    }
    for step in update_step(
        "update-failed",
        || AutoupdateStage::NoUpdateAvailable,
        None,
        Some(ClinchUpdateFailure::Download),
    ) {
        builder = builder.with_step(step);
    }
    builder = builder.with_step(workspace_action(
        "close the update pop-up",
        WorkspaceAction::CloseClinchUpdatePopup,
    ));
    builder
        .with_step(shot("footer-before-drag"))
        .with_step(drag_footer_button(
            "drag the second button to the front",
            1,
            0,
            -8.,
        ))
        .with_step(shot("footer-reordered"))
        .with_step(
            TestStep::new("open More")
                .with_click_on_saved_position("agent_footer:more")
                .set_post_step_pause(std::time::Duration::from_millis(400)),
        )
        .with_step(shot("footer-more-open"))
        .with_step(drag_more_button_to_row("drag a button out of More", 1))
        .with_step(shot("footer-after-dragging-out"))
        .with_step(
            TestStep::new("close More")
                .with_click_on_saved_position("agent_footer:more")
                .set_post_step_pause(std::time::Duration::from_millis(400)),
        )
        .with_step(drag_row_button_to_more("drag a button into More", 0))
        .with_step(shot("footer-after-dragging-in"))
}

/// Drags a row button onto the closed More button.
fn drag_row_button_to_more(name: &str, from: usize) -> TestStep {
    const STEPS: usize = 12;
    let button = move |app: &mut App, window_id: WindowId| {
        let terminal = single_terminal_view_for_tab(app, window_id, 0).id();
        saved_center(
            app,
            window_id,
            &format!("agent_footer:{terminal}:button:{from}"),
        )
    };
    let more = |app: &mut App, window_id: WindowId| {
        let terminal = single_terminal_view_for_tab(app, window_id, 0).id();
        saved_center(app, window_id, &format!("agent_footer:{terminal}:more"))
    };
    let mut step = TestStep::new(name).with_event_fn(move |app, window_id| Event::LeftMouseDown {
        position: button(app, window_id),
        modifiers: ModifiersState::default(),
        click_count: 1,
        is_first_mouse: false,
    });
    for i in 1..=STEPS {
        step = step.with_event_fn(move |app, window_id| {
            let (from, to) = (button(app, window_id), more(app, window_id));
            Event::LeftMouseDragged {
                position: from + (to - from) * (i as f32 / STEPS as f32),
                modifiers: ModifiersState::default(),
            }
        });
    }
    step.with_event_fn(move |app, window_id| Event::LeftMouseUp {
        position: more(app, window_id),
        modifiers: ModifiersState::default(),
    })
    .set_post_step_pause(std::time::Duration::from_millis(500))
}

/// Presses a footer button in the first tab and drags it onto another, the way a hand does.
fn drag_footer_button(name: &str, from: usize, to: usize, dx: f32) -> TestStep {
    const STEPS: usize = 12;
    let id = |app: &App, window_id: WindowId, index: usize| {
        let terminal = single_terminal_view_for_tab(app, window_id, 0).id();
        format!("agent_footer:{terminal}:button:{index}")
    };
    let mut step = TestStep::new(name).with_event_fn(move |app, window_id| {
        let from = id(app, window_id, from);
        Event::LeftMouseDown {
            position: saved_center(app, window_id, &from),
            modifiers: ModifiersState::default(),
            click_count: 1,
            is_first_mouse: false,
        }
    });
    for i in 1..=STEPS {
        step = step.with_event_fn(move |app, window_id| {
            let (from, to) = (id(app, window_id, from), id(app, window_id, to));
            let start = saved_center(app, window_id, &from);
            let end = saved_center(app, window_id, &to) + vec2f(dx, 0.);
            Event::LeftMouseDragged {
                position: start + (end - start) * (i as f32 / STEPS as f32),
                modifiers: ModifiersState::default(),
            }
        });
    }
    step.with_event_fn(move |app, window_id| {
        let to = id(app, window_id, to);
        Event::LeftMouseUp {
            position: saved_center(app, window_id, &to) + vec2f(dx, 0.),
            modifiers: ModifiersState::default(),
        }
    })
    .set_post_step_pause(std::time::Duration::from_millis(500))
}

/// Drags the button at `row` buttons past the first one under More down onto the footer's row.
fn drag_more_button_to_row(name: &str, row: usize) -> TestStep {
    const STEPS: usize = 12;
    let more_menu = |app: &mut App, window_id: WindowId| {
        let terminal = single_terminal_view_for_tab(app, window_id, 0).id();
        saved_rect(
            app,
            window_id,
            &format!("agent_footer:{terminal}:more_menu"),
        )
    };
    let start = move |app: &mut App, window_id: WindowId| {
        let menu = more_menu(app, window_id);
        // The menu's buttons are about 34px tall, inside 8px of padding.
        menu.origin() + vec2f(40., 8. + 17. + row as f32 * 34.)
    };
    let end = |app: &mut App, window_id: WindowId| {
        let terminal = single_terminal_view_for_tab(app, window_id, 0).id();
        saved_center(app, window_id, &format!("agent_footer:{terminal}:button:0")) + vec2f(-8., 0.)
    };
    let mut step = TestStep::new(name).with_event_fn(move |app, window_id| Event::LeftMouseDown {
        position: start(app, window_id),
        modifiers: ModifiersState::default(),
        click_count: 1,
        is_first_mouse: false,
    });
    for i in 1..=STEPS {
        step = step.with_event_fn(move |app, window_id| {
            let (from, to) = (start(app, window_id), end(app, window_id));
            Event::LeftMouseDragged {
                position: from + (to - from) * (i as f32 / STEPS as f32),
                modifiers: ModifiersState::default(),
            }
        });
    }
    step.with_event_fn(move |app, window_id| Event::LeftMouseUp {
        position: end(app, window_id),
        modifiers: ModifiersState::default(),
    })
    .set_post_step_pause(std::time::Duration::from_millis(500))
}

fn saved_rect(app: &mut App, window_id: WindowId, id: &str) -> pathfinder_geometry::rect::RectF {
    app.presenter(window_id)
        .expect("presenter")
        .borrow()
        .position_cache()
        .get_position(id)
        .unwrap_or_else(|| panic!("{id} was not laid out"))
}

fn saved_center(app: &mut App, window_id: WindowId, id: &str) -> Vector2F {
    app.presenter(window_id)
        .expect("presenter")
        .borrow()
        .position_cache()
        .get_position(id)
        .unwrap_or_else(|| panic!("{id} was not laid out"))
        .center()
}

/// Presses on one saved element, moves in small steps to another (offset vertically by `dy`),
/// and releases there, the way a hand drags.
fn drag(name: &str, from: &'static str, to: &'static str, dy: f32) -> TestStep {
    const STEPS: usize = 12;
    let mut step = TestStep::new(name).with_event_fn(move |app, window_id| Event::LeftMouseDown {
        position: saved_center(app, window_id, from),
        modifiers: ModifiersState::default(),
        click_count: 1,
        is_first_mouse: false,
    });
    for i in 1..=STEPS {
        step = step.with_event_fn(move |app, window_id| {
            let start = saved_center(app, window_id, from);
            let end = saved_center(app, window_id, to) + vec2f(0., dy);
            Event::LeftMouseDragged {
                position: start + (end - start) * (i as f32 / STEPS as f32),
                modifiers: ModifiersState::default(),
            }
        });
    }
    step.with_event_fn(move |app, window_id| Event::LeftMouseUp {
        position: saved_center(app, window_id, to) + vec2f(0., dy),
        modifiers: ModifiersState::default(),
    })
    .set_post_step_pause(std::time::Duration::from_millis(500))
}

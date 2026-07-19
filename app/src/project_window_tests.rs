use warpui::keymap::Keystroke;
use warpui::platform::OperatingSystem;
use warpui::App;

use super::{
    active_index_after_removal, close_project_decision, next_project_index, previous_project_index,
    project_agent_hover_summary, CloseProjectDecision, ACTIVATE_NEXT_PROJECT_MAC_KEY_BINDING,
    ACTIVATE_PREVIOUS_PROJECT_MAC_KEY_BINDING, PROJECT_TAB_BORDER_WIDTH,
    PROJECT_TAB_VERTICAL_NUDGE, PROJECT_TAB_VERTICAL_PADDING,
};
use crate::util::bindings::{custom_tag_to_keystroke, trigger_to_keystroke, CustomAction};
use crate::workspace::view::ProjectCliAgentCounts;

#[test]
fn project_navigation_owns_command_brackets_on_mac() {
    App::test((), |mut app| async move {
        app.update(super::init);

        app.update(|ctx| {
            let previous = ctx
                .editable_bindings()
                .find(|binding| binding.name == "project_window:activate_previous_project")
                .and_then(|binding| trigger_to_keystroke(binding.trigger));
            let next = ctx
                .editable_bindings()
                .find(|binding| binding.name == "project_window:activate_next_project")
                .and_then(|binding| trigger_to_keystroke(binding.trigger));

            if OperatingSystem::get().is_mac() {
                assert_eq!(
                    previous,
                    Keystroke::parse(ACTIVATE_PREVIOUS_PROJECT_MAC_KEY_BINDING).ok()
                );
                assert_eq!(
                    next,
                    Keystroke::parse(ACTIVATE_NEXT_PROJECT_MAC_KEY_BINDING).ok()
                );
                assert_eq!(
                    custom_tag_to_keystroke(CustomAction::ActivatePreviousPane.into()),
                    None
                );
                assert_eq!(
                    custom_tag_to_keystroke(CustomAction::ActivateNextPane.into()),
                    None
                );
            } else {
                assert_eq!(previous, None);
                assert_eq!(next, None);
                assert_eq!(
                    custom_tag_to_keystroke(CustomAction::ActivatePreviousPane.into()),
                    Keystroke::parse("ctrl-shift-{").ok()
                );
                assert_eq!(
                    custom_tag_to_keystroke(CustomAction::ActivateNextPane.into()),
                    Keystroke::parse("ctrl-shift-}").ok()
                );
            }
        });
    });
}

#[test]
fn project_navigation_wraps_and_singletons_are_noops() {
    assert_eq!(previous_project_index(0, 3), Some(2));
    assert_eq!(previous_project_index(2, 3), Some(1));
    assert_eq!(next_project_index(2, 3), Some(0));
    assert_eq!(next_project_index(0, 3), Some(1));

    assert_eq!(previous_project_index(0, 1), None);
    assert_eq!(next_project_index(0, 1), None);
}

#[test]
fn project_agent_hover_summary_matches_project_badge_counts() {
    assert_eq!(
        project_agent_hover_summary(
            3,
            2,
            ProjectCliAgentCounts {
                working: 1,
                done: 1,
            },
        ),
        "3 open tabs · 2 agents · 1 working · 1 done"
    );
    assert_eq!(
        project_agent_hover_summary(1, 0, ProjectCliAgentCounts::default()),
        "1 open tab"
    );
}

#[test]
fn closing_active_project_prefers_the_project_at_the_same_position() {
    assert_eq!(active_index_after_removal(1, 1, 2), Some(1));
    assert_eq!(active_index_after_removal(2, 2, 2), Some(1));
}

#[test]
fn removing_inactive_project_preserves_active_project_identity() {
    assert_eq!(active_index_after_removal(2, 0, 2), Some(1));
    assert_eq!(active_index_after_removal(0, 2, 2), Some(0));
    assert_eq!(active_index_after_removal(0, 0, 0), None);
}

#[test]
fn close_project_guard_distinguishes_missing_singleton_and_grouped_projects() {
    assert_eq!(
        close_project_decision(2, None),
        CloseProjectDecision::NotFound
    );
    assert_eq!(
        close_project_decision(1, Some(0)),
        CloseProjectDecision::CloseWindow
    );
    assert_eq!(
        close_project_decision(3, Some(1)),
        CloseProjectDecision::Project(1)
    );
}

/// Mirrors the tab strip's vertical composition inside the title bar. `Text`
/// drops a single-line label entirely (not just clips it) when the line height
/// exceeds its max-height constraint, so if this budget dips below one UI line
/// the project tabs render as blank pills. Note the strip's `ClippedScrollable`
/// must keep zero scrollbar gutter padding (see `render_project_tab_strip`) or
/// 4px silently disappear from this budget.
#[test]
fn project_tab_label_height_budget_fits_one_ui_line() {
    let label_budget = crate::workspace::view::TAB_BAR_HEIGHT
        - PROJECT_TAB_VERTICAL_NUDGE
        - 2. * (PROJECT_TAB_VERTICAL_PADDING + PROJECT_TAB_BORDER_WIDTH);
    let ui_line_height = warp_core::ui::appearance::DEFAULT_UI_FONT_SIZE
        * warpui::elements::DEFAULT_UI_LINE_HEIGHT_RATIO;
    assert!(
        label_budget >= ui_line_height,
        "project tab label budget ({label_budget}px) no longer fits one UI line \
         ({ui_line_height}px); the labels will disappear entirely"
    );
}

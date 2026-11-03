use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;
use warp_core::ui::tokens::{StatusKind, Tokens};
use warpui::elements::PositionedElementOffsetBounds;
use warpui::EntityId;

use super::{
    code_detail_kind_label, detail_sidecar_width_and_bounds, detail_target_for_hovered_row,
    drag_autoscroll_step, non_terminal_search_text_fragments, pane_ids_for_display_granularity,
    pane_search_text_fragments, path_belongs_to_project, preferred_agent_tab_titles,
    preferred_vertical_tab_title_override, row_renames_pane, search_fragments_contain_query,
    should_keep_detail_sidecar_visible_for_mouse_position, tab_title_uses_header,
    terminal_command_status, terminal_kind_badge_label, terminal_primary_line_data,
    terminal_pull_request_badge_label, terminal_search_text_fragments,
    terminal_title_fallback_font, uses_outer_group_container, visible_pane_ids_for_detail_target,
    vtab_diff_stats_text, AgentTabTextPreference, TerminalAgentText, TerminalPrimaryLineData,
    TerminalPrimaryLineFont, VerticalTabsDetailTarget, VerticalTabsDetailTargetKind,
};
use crate::ai::agent::conversation::ConversationStatus;
use crate::context_chips::display_chip::GitLineChanges;
use crate::pane_group::pane::IPaneType;
use crate::pane_group::{PaneId, TerminalPaneId};
use crate::safe_triangle::SafeTriangle;
use crate::terminal::view::TerminalViewState;
use crate::terminal::CLIAgent;
use crate::workspace::tab_settings::VerticalTabsDisplayGranularity;
use crate::workspace::view::ProjectCliAgentActivity;

#[test]
fn bookmarked_session_paths_are_scoped_to_the_current_project() {
    let project = std::path::Path::new("/work/current-project");

    assert!(path_belongs_to_project(project, project));
    assert!(path_belongs_to_project(
        std::path::Path::new("/work/current-project/crates/app"),
        project
    ));
    assert!(!path_belongs_to_project(
        std::path::Path::new("/work/other-project"),
        project
    ));
    assert!(!path_belongs_to_project(
        std::path::Path::new("/work/current-project-old"),
        project
    ));
}

fn pane_id() -> PaneId {
    TerminalPaneId::dummy_terminal_pane_id().into()
}

#[test]
fn terminal_command_status_is_in_progress_only_while_command_is_running() {
    assert_eq!(
        terminal_command_status(TerminalViewState::LongRunning),
        Some(ConversationStatus::InProgress)
    );
    assert_eq!(terminal_command_status(TerminalViewState::Normal), None);
    assert_eq!(terminal_command_status(TerminalViewState::Errored), None);
}

#[test]
fn agent_activity_dots_match_project_tab_working_and_done_colors() {
    let theme = crate::themes::default_themes::dark_theme();
    let tokens = Tokens::new(&theme);

    assert_eq!(
        ProjectCliAgentActivity::Working.color(&tokens),
        tokens.status(StatusKind::Working)
    );
    assert_eq!(
        ProjectCliAgentActivity::Done.color(&tokens),
        tokens.status(StatusKind::Done)
    );
    assert_eq!(
        ProjectCliAgentActivity::NeedsAttention.color(&tokens),
        tokens.status(StatusKind::Done)
    );
    assert_eq!(
        ProjectCliAgentActivity::NeedsInput.color(&tokens),
        tokens.status(StatusKind::NeedsYou)
    );
    assert_eq!(
        ProjectCliAgentActivity::RateLimited.color(&tokens),
        tokens.status(StatusKind::RateLimited)
    );
    assert_eq!(ProjectCliAgentActivity::Idle.color(&tokens), tokens.text_3);
}

#[test]
fn preferred_agent_tab_titles_default_to_title_like_text() {
    let agent_text = TerminalAgentText {
        conversation_display_title: Some("Generated Oz title".to_string()),
        conversation_latest_user_prompt: Some("Latest Oz prompt".to_string()),
        cli_agent_title: Some("CLI summary".to_string()),
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: true,
        cli_agent: Some(CLIAgent::Claude),
    };

    assert_eq!(
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::ConversationTitle),
        (
            Some("Generated Oz title".to_string()),
            Some("CLI summary".to_string())
        )
    );
}

#[test]
fn preferred_agent_tab_titles_do_not_use_cli_prompt_when_disabled() {
    let agent_text = TerminalAgentText {
        conversation_display_title: None,
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: false,
        cli_agent: Some(CLIAgent::Claude),
    };

    assert_eq!(
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::ConversationTitle),
        (None, None)
    );
}

#[test]
fn terminal_primary_line_uses_terminal_title_when_disabled_cli_has_only_prompt() {
    let agent_text = TerminalAgentText {
        conversation_display_title: None,
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: false,
        cli_agent: Some(CLIAgent::Claude),
    };
    let (conversation_title, cli_title) =
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::ConversationTitle);

    let line = terminal_primary_line_data(
        false,
        conversation_title,
        cli_title,
        "Generated Claude Code title",
        "~/warp",
        terminal_title_fallback_font(&agent_text),
        Some("claude".to_string()),
    );

    assert_eq!(line.text(), "Generated Claude Code title");
    assert!(matches!(
        line,
        TerminalPrimaryLineData::Text {
            font: TerminalPrimaryLineFont::Ui,
            ..
        }
    ));
}

#[test]
fn preferred_agent_tab_titles_use_latest_prompt_when_enabled() {
    let agent_text = TerminalAgentText {
        conversation_display_title: Some("Generated Oz title".to_string()),
        conversation_latest_user_prompt: Some("Latest Oz prompt".to_string()),
        cli_agent_title: Some("CLI summary".to_string()),
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: true,
        cli_agent: Some(CLIAgent::Claude),
    };

    assert_eq!(
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::LatestUserPrompt),
        (
            Some("Latest Oz prompt".to_string()),
            Some("Latest CLI prompt".to_string())
        )
    );
}

#[test]
fn terminal_primary_line_uses_cli_prompt_when_enabled_cli_has_prompt() {
    let agent_text = TerminalAgentText {
        conversation_display_title: None,
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: false,
        cli_agent: Some(CLIAgent::Claude),
    };
    let (conversation_title, cli_title) =
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::LatestUserPrompt);

    let line = terminal_primary_line_data(
        false,
        conversation_title,
        cli_title,
        "Generated Claude Code title",
        "~/warp",
        terminal_title_fallback_font(&agent_text),
        Some("claude".to_string()),
    );

    assert_eq!(line.text(), "Latest CLI prompt");
}

#[test]
fn terminal_primary_line_uses_cli_prompt_when_enabled_cli_is_long_running() {
    let agent_text = TerminalAgentText {
        conversation_display_title: None,
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: false,
        cli_agent: Some(CLIAgent::Claude),
    };
    let (conversation_title, cli_title) =
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::LatestUserPrompt);

    let line = terminal_primary_line_data(
        true,
        conversation_title,
        cli_title,
        "Generated Claude Code title",
        "~/warp",
        terminal_title_fallback_font(&agent_text),
        Some("claude".to_string()),
    );

    assert_eq!(line.text(), "Latest CLI prompt");
}

#[test]
fn preferred_agent_tab_titles_fall_back_when_preferred_text_is_missing() {
    let agent_text = TerminalAgentText {
        conversation_display_title: Some("Generated Oz title".to_string()),
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: true,
        cli_agent: Some(CLIAgent::Claude),
    };

    assert_eq!(
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::LatestUserPrompt),
        (
            Some("Generated Oz title".to_string()),
            Some("Latest CLI prompt".to_string())
        )
    );
}

fn pane_type_supports_vertical_tabs_detail_sidecar(pane_type: IPaneType) -> bool {
    matches!(
        pane_type,
        IPaneType::Terminal
            | IPaneType::Code
            | IPaneType::Notebook
            | IPaneType::Workflow
            | IPaneType::EnvVarCollection
            | IPaneType::AIFact
            | IPaneType::AIDocument
    )
}

fn collect_normalized_unique_summary_texts(
    texts: impl IntoIterator<Item = impl AsRef<str>>,
) -> Vec<String> {
    texts
        .into_iter()
        .filter_map(|text| {
            let normalized = text
                .as_ref()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            (!normalized.is_empty()).then_some(normalized)
        })
        .fold(Vec::new(), |mut values, normalized| {
            if !values.contains(&normalized) {
                values.push(normalized);
            }
            values
        })
}

#[test]
fn detail_sidecar_supports_terminal_code_and_warp_drive_object_panes() {
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Terminal
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Code
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Notebook
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Workflow
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::EnvVarCollection
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::AIFact
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::AIDocument
    ));
    assert!(!pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Settings
    ));
}

#[test]
fn code_detail_kind_label_uses_programming_language_display_name() {
    assert_eq!(
        code_detail_kind_label("block_id.rs"),
        Some("Rust".to_string())
    );
    assert_eq!(
        code_detail_kind_label("Dockerfile"),
        Some("Dockerfile".to_string())
    );
}

#[test]
fn code_detail_kind_label_returns_none_when_language_is_unknown() {
    assert_eq!(code_detail_kind_label("notes.txt"), None);
}

#[test]
fn detail_target_matches_panes_granularity() {
    let pane_group_id = EntityId::new();
    let hovered_pane_id = pane_id();

    assert_eq!(
        detail_target_for_hovered_row(
            pane_group_id,
            hovered_pane_id,
            VerticalTabsDisplayGranularity::Panes,
        ),
        VerticalTabsDetailTarget::Pane {
            pane_group_id,
            pane_id: hovered_pane_id,
        }
    );
}

#[test]
fn detail_target_matches_tabs_granularity() {
    let pane_group_id = EntityId::new();
    let hovered_pane_id = pane_id();

    assert_eq!(
        detail_target_for_hovered_row(
            pane_group_id,
            hovered_pane_id,
            VerticalTabsDisplayGranularity::Tabs,
        ),
        VerticalTabsDetailTarget::Tab {
            pane_group_id,
            source_pane_id: hovered_pane_id,
        }
    );
}

#[test]
fn pane_detail_target_returns_hovered_pane_when_supported() {
    let hovered_pane_id = pane_id();

    assert_eq!(
        visible_pane_ids_for_detail_target(
            &[hovered_pane_id],
            hovered_pane_id,
            VerticalTabsDetailTargetKind::Pane,
            |pane_id| pane_id == hovered_pane_id,
        ),
        Some(vec![hovered_pane_id])
    );
}

#[test]
fn pane_detail_target_returns_none_when_hovered_pane_is_not_supported() {
    let hovered_pane_id = pane_id();

    assert_eq!(
        visible_pane_ids_for_detail_target(
            &[hovered_pane_id],
            hovered_pane_id,
            VerticalTabsDetailTargetKind::Pane,
            |_| false,
        ),
        None
    );
}

#[test]
fn tab_detail_target_returns_all_visible_panes_when_every_pane_is_supported() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();
    let visible_pane_ids = vec![pane_1, pane_2, pane_3];

    assert_eq!(
        visible_pane_ids_for_detail_target(
            &visible_pane_ids,
            pane_2,
            VerticalTabsDetailTargetKind::Tab,
            |_| true,
        ),
        Some(visible_pane_ids)
    );
}

#[test]
fn tab_detail_target_returns_none_for_mixed_support_tabs() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();

    assert_eq!(
        visible_pane_ids_for_detail_target(
            &[pane_1, pane_2, pane_3],
            pane_2,
            VerticalTabsDetailTargetKind::Tab,
            |pane_id| pane_id != pane_3,
        ),
        None
    );
}

#[test]
fn panes_granularity_returns_all_visible_panes_in_order() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();
    let visible_pane_ids = vec![pane_1, pane_2, pane_3];

    assert_eq!(
        pane_ids_for_display_granularity(
            &visible_pane_ids,
            pane_2,
            VerticalTabsDisplayGranularity::Panes,
        ),
        visible_pane_ids
    );
}

#[test]
fn tabs_granularity_returns_focused_pane_when_present() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();

    assert_eq!(
        pane_ids_for_display_granularity(
            &[pane_1, pane_2, pane_3],
            pane_2,
            VerticalTabsDisplayGranularity::Tabs,
        ),
        vec![pane_2]
    );
}

#[test]
fn tabs_granularity_falls_back_to_first_visible_pane_when_focused_pane_is_absent() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();
    let focused_pane = pane_id();

    assert_eq!(
        pane_ids_for_display_granularity(
            &[pane_1, pane_2, pane_3],
            focused_pane,
            VerticalTabsDisplayGranularity::Tabs,
        ),
        vec![pane_1]
    );
}

#[test]
fn tabs_granularity_returns_empty_for_empty_visible_panes() {
    assert_eq!(
        pane_ids_for_display_granularity(&[], pane_id(), VerticalTabsDisplayGranularity::Tabs,),
        Vec::<PaneId>::new()
    );
}

#[test]
fn detail_sidecar_uses_default_width_when_space_allows() {
    let (width, bounds) = detail_sidecar_width_and_bounds(400.);
    assert_eq!(width, 320.);
    assert!(matches!(
        bounds,
        PositionedElementOffsetBounds::WindowBySize
    ));
}

#[test]
fn detail_sidecar_shrinks_to_fit_before_hitting_min_width() {
    let (width, bounds) = detail_sidecar_width_and_bounds(280.);
    assert_eq!(width, 280.);
    assert!(matches!(
        bounds,
        PositionedElementOffsetBounds::WindowBySize
    ));
}

#[test]
fn detail_sidecar_stops_shrinking_at_min_width_and_allows_clipping() {
    let (width, bounds) = detail_sidecar_width_and_bounds(180.);
    assert_eq!(width, 240.);
    assert!(matches!(bounds, PositionedElementOffsetBounds::Unbounded));
}

#[test]
fn detail_sidecar_visibility_helper_keeps_sidecar_visible_inside_sidecar_bounds() {
    let row_rect = RectF::new(Vector2F::new(0., 100.), Vector2F::new(100., 40.));
    let sidecar_rect = RectF::new(Vector2F::new(120., 50.), Vector2F::new(180., 220.));
    let mut safe_triangle = SafeTriangle::new();

    assert!(should_keep_detail_sidecar_visible_for_mouse_position(
        Vector2F::new(200., 120.),
        Some(row_rect),
        Some(sidecar_rect),
        &mut safe_triangle,
    ));
}

#[test]
fn detail_sidecar_visibility_helper_keeps_sidecar_visible_in_safe_triangle() {
    let row_rect = RectF::new(Vector2F::new(0., 100.), Vector2F::new(100., 40.));
    let sidecar_rect = RectF::new(Vector2F::new(120., 50.), Vector2F::new(180., 220.));
    let mut safe_triangle = SafeTriangle::new();
    safe_triangle.set_target_rect(Some(sidecar_rect));
    safe_triangle.update_position(Vector2F::new(90., 120.));

    assert!(should_keep_detail_sidecar_visible_for_mouse_position(
        Vector2F::new(110., 120.),
        Some(row_rect),
        Some(sidecar_rect),
        &mut safe_triangle,
    ));
}

#[test]
fn detail_sidecar_visibility_helper_clears_sidecar_outside_row_sidecar_and_safe_triangle() {
    let row_rect = RectF::new(Vector2F::new(0., 100.), Vector2F::new(100., 40.));
    let sidecar_rect = RectF::new(Vector2F::new(120., 50.), Vector2F::new(180., 220.));
    let mut safe_triangle = SafeTriangle::new();
    safe_triangle.update_position(Vector2F::new(200., 120.));

    assert!(!should_keep_detail_sidecar_visible_for_mouse_position(
        Vector2F::new(340., 120.),
        Some(row_rect),
        Some(sidecar_rect),
        &mut safe_triangle,
    ));
}

#[test]
fn panes_granularity_uses_outer_group_container() {
    assert!(uses_outer_group_container(
        VerticalTabsDisplayGranularity::Panes
    ));
}

#[test]
fn tabs_granularity_does_not_use_outer_group_container() {
    assert!(!uses_outer_group_container(
        VerticalTabsDisplayGranularity::Tabs
    ));
}

#[test]
fn terminal_primary_line_prefers_cli_agent_display_title() {
    let line = terminal_primary_line_data(
        false,
        None,
        Some("Review the failing tests".to_string()),
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "Review the failing tests");
}

#[test]
fn terminal_primary_line_prefers_cli_agent_display_title_over_conversation_title() {
    let line = terminal_primary_line_data(
        false,
        Some("Review the failing tests".to_string()),
        Some("Summarize the failures".to_string()),
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "Summarize the failures");
}

#[test]
fn terminal_primary_line_falls_through_to_terminal_title_when_cli_agent_has_no_plugin_data() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "codex - ~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "codex - ~/warp");
}

#[test]
fn terminal_primary_line_uses_terminal_title_as_fallback() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "nvim src/workspace/view/vertical_tabs.rs",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "nvim src/workspace/view/vertical_tabs.rs");
}

#[test]
fn terminal_primary_line_uses_last_completed_command_when_shell_title_matches_working_directory() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "cargo nextest run");
}

#[test]
fn terminal_primary_line_falls_back_to_new_session() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        None,
    );

    assert_eq!(line.text(), "New session");
    assert!(matches!(
        line,
        TerminalPrimaryLineData::Text {
            font: TerminalPrimaryLineFont::Ui,
            ..
        }
    ));
}

#[test]
fn terminal_primary_line_uses_monospace_for_last_completed_command() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert!(matches!(
        line,
        TerminalPrimaryLineData::Text {
            font: TerminalPrimaryLineFont::Monospace,
            ..
        }
    ));
}

#[test]
fn terminal_search_fragments_include_rendered_terminal_badges() {
    let fragments = terminal_search_text_fragments(
        "Review the failing tests".to_string(),
        "~/warp".to_string(),
        Some("main".to_string()),
        terminal_kind_badge_label(false, Some(CLIAgent::Claude)),
        true,
        Some(terminal_pull_request_badge_label(
            "https://github.com/warpdotdev/warp-internal/pull/12345",
        )),
        Some(GitLineChanges {
            files_changed: 1,
            lines_added: 2,
            lines_removed: 3,
        }),
    );

    assert!(search_fragments_contain_query(&fragments, "claude"));
    assert!(search_fragments_contain_query(
        &fragments,
        "review the failing tests"
    ));
    assert!(search_fragments_contain_query(&fragments, "#12345"));
    assert!(search_fragments_contain_query(&fragments, "worktree"));
    assert!(search_fragments_contain_query(&fragments, "+2"));
    assert!(search_fragments_contain_query(&fragments, "-3"));
}

#[test]
fn pane_search_fragments_prepend_custom_title_and_keep_generated_metadata() {
    let fragments = pane_search_text_fragments(
        Some("Production API"),
        vec![
            "cargo nextest run".to_string(),
            "~/warp".to_string(),
            "Claude".to_string(),
        ],
    );

    assert_eq!(fragments[0], "Production API");
    assert!(search_fragments_contain_query(&fragments, "production api"));
    assert!(search_fragments_contain_query(&fragments, "cargo nextest"));
    assert!(search_fragments_contain_query(&fragments, "~/warp"));
    assert!(search_fragments_contain_query(&fragments, "claude"));
}

#[test]
fn pane_search_fragments_dedupe_custom_title_against_generated_text() {
    assert_eq!(
        pane_search_text_fragments(
            Some("  Production   API  "),
            vec![
                "Production API".to_string(),
                "~/warp".to_string(),
                "~/warp".to_string(),
            ],
        ),
        vec!["Production API".to_string(), "~/warp".to_string()]
    );
}

#[test]
fn single_pane_tab_names_replace_generated_titles() {
    assert!(!tab_title_uses_header(
        VerticalTabsDisplayGranularity::Panes,
        1
    ));
    assert_eq!(
        preferred_vertical_tab_title_override(Some("My renamed tab"), None),
        Some("My renamed tab")
    );
    assert_eq!(
        preferred_vertical_tab_title_override(Some("My renamed tab"), Some("My pane name")),
        Some("My renamed tab")
    );
}

#[test]
fn split_tab_names_use_a_header_without_replacing_pane_titles() {
    assert!(tab_title_uses_header(
        VerticalTabsDisplayGranularity::Panes,
        2
    ));
    assert!(!tab_title_uses_header(
        VerticalTabsDisplayGranularity::Tabs,
        2
    ));
}

#[test]
fn double_click_renames_exactly_one_target() {
    // Rows that display the tab name (single-pane tabs, Tabs view) rename the tab;
    // only split-tab pane rows rename the pane. Doing both from one double-click
    // let the losing editor commit the old name, so a rename appeared to revert.
    assert!(!row_renames_pane(
        VerticalTabsDisplayGranularity::Panes,
        Some(0)
    ));
    assert!(row_renames_pane(
        VerticalTabsDisplayGranularity::Panes,
        None
    ));
    assert!(!row_renames_pane(
        VerticalTabsDisplayGranularity::Tabs,
        Some(0)
    ));
}

#[test]
fn non_terminal_search_fragments_only_include_rendered_text() {
    let fragments = non_terminal_search_text_fragments("Pane title", "and 2 more");

    assert!(search_fragments_contain_query(&fragments, "pane title"));
    assert!(search_fragments_contain_query(&fragments, "and 2 more"));
    assert!(!search_fragments_contain_query(&fragments, "notebook"));
    assert!(!search_fragments_contain_query(&fragments, "unsaved"));
}

#[test]
fn diff_stats_text_matches_rendered_badge_text() {
    assert_eq!(
        vtab_diff_stats_text(&GitLineChanges {
            files_changed: 1,
            lines_added: 2,
            lines_removed: 3,
        }),
        "+2 -3"
    );
    assert_eq!(
        vtab_diff_stats_text(&GitLineChanges {
            files_changed: 1,
            lines_added: 0,
            lines_removed: 0,
        }),
        "0"
    );
}

#[test]
fn collect_normalized_unique_summary_texts_dedupes_after_whitespace_normalization() {
    assert_eq!(
        collect_normalized_unique_summary_texts([
            "  cargo   test  ",
            "cargo test",
            "",
            " git   status ",
        ]),
        vec!["cargo test".to_string(), "git status".to_string()]
    );
}

#[test]
fn collect_normalized_unique_summary_texts_preserves_first_seen_order() {
    assert_eq!(
        collect_normalized_unique_summary_texts([
            "~/warp-internal",
            "~/warp-server",
            "~/warp-internal",
            "~/warp-terraform",
        ]),
        vec![
            "~/warp-internal".to_string(),
            "~/warp-server".to_string(),
            "~/warp-terraform".to_string(),
        ]
    );
}

#[test]
fn drag_autoscroll_scrolls_toward_the_nearer_edge_and_ramps_with_depth() {
    // A 500px-tall tab list starting 100px down the window.
    let viewport = RectF::new(Vector2F::new(0., 100.), Vector2F::new(240., 500.));

    // Middle of the list: no scroll.
    assert_eq!(drag_autoscroll_step(viewport, 350.), 0.);
    // Top band scrolls up, faster the closer to the edge.
    let shallow_up = drag_autoscroll_step(viewport, 140.);
    let deep_up = drag_autoscroll_step(viewport, 105.);
    assert!(shallow_up < 0. && deep_up < shallow_up);
    // Bottom band scrolls down.
    assert!(drag_autoscroll_step(viewport, 590.) > 0.);
    // Just past an edge (over the control bar) keeps scrolling at full speed...
    assert_eq!(
        drag_autoscroll_step(viewport, 90.),
        drag_autoscroll_step(viewport, 60.)
    );
    assert!(drag_autoscroll_step(viewport, 90.) < 0.);
    // ...but a drag well away from the list does not scroll it.
    assert_eq!(drag_autoscroll_step(viewport, 0.), 0.);
    assert_eq!(drag_autoscroll_step(viewport, 700.), 0.);
}

#[test]
fn drag_autoscroll_ignores_collapsed_list() {
    let viewport = RectF::new(Vector2F::new(0., 100.), Vector2F::new(240., 0.));
    assert_eq!(drag_autoscroll_step(viewport, 100.), 0.);
}

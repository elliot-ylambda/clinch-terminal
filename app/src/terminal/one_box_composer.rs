//! One-box agent input: Clinch's input box takes the place of a Claude Code or Codex composer.
//!
//! The agent keeps drawing its whole screen. While its composer is visible and empty, the grid
//! element paints over the composer rows with the terminal background and records their rectangle;
//! `TerminalView` then positions the rich input over that rectangle in the same frame. Everything
//! else the agent draws stays visible: the transcript and subagents above, and the status line,
//! mode row and background-task counts below.
//!
//! When the composer is not readable as empty (a permission prompt, a picker, a dialog, or text the
//! agent put in its own box), nothing is covered and the rectangle is not recorded, so the
//! conditionally anchored overlay is skipped and the agent's own screen shows through.
use std::ops::RangeInclusive;

use warpui::elements::Fill;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::{vec2f, Vector2F};
use warpui::{AppContext, ClipBounds, EntityId, PaintContext, SingletonEntity as _};

use crate::settings::AISettings;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::model::grid::grid_handler::GridHandler;
use crate::terminal::view::use_agent_footer::native_composer_area;
use crate::terminal::{CLIAgent, TerminalModel};

/// Agents whose composer Clinch can find on screen, and so replace.
pub(crate) fn supports_one_box(agent: CLIAgent) -> bool {
    matches!(agent, CLIAgent::Claude | CLIAgent::Codex)
}

/// Whether a new `agent` session should open the rich input as soon as its composer shows.
pub(crate) fn opens_rich_input_on_start(agent: CLIAgent, app: &AppContext) -> bool {
    let settings = AISettings::as_ref(app);
    *settings.auto_open_rich_input_on_cli_agent_start
        || (*settings.one_box_agent_input && supports_one_box(agent))
}

/// The agent whose composer Clinch's open input box replaces in this pane, if any.
pub(crate) fn one_box_agent(
    model: &TerminalModel,
    terminal_view_id: EntityId,
    app: &AppContext,
) -> Option<CLIAgent> {
    if !*AISettings::as_ref(app).one_box_agent_input
        || !model
            .block_list()
            .active_block()
            .is_active_and_long_running()
    {
        return None;
    }
    let sessions = CLIAgentSessionsModel::as_ref(app);
    if !sessions.is_input_open(terminal_view_id) {
        return None;
    }
    let session = sessions.session(terminal_view_id)?;
    (supports_one_box(session.agent) && !session.is_remote()).then_some(session.agent)
}

/// Agent keys that pass through Clinch's box in one-box mode. Clinch's own menus take them first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OneBoxKey {
    /// Interrupts a turn; twice opens Claude Code's rewind picker.
    Escape,
    /// Cycles the permission mode.
    ShiftTab,
    /// Claude Code's transcript view.
    CtrlO,
    /// Claude Code's task list.
    CtrlT,
    /// The agent's own prompt history, when Clinch knows none of the session's messages; in a slash
    /// command, the agent's command menu.
    Up,
    /// The agent's command menu, while the box holds a slash command. So are the keys below.
    Down,
    /// Completes the command chosen in the agent's menu.
    Tab,
    /// Runs the command chosen in the agent's menu.
    Enter,
}

impl OneBoxKey {
    /// The keys among those the editor has no binding for.
    pub(crate) fn from_unhandled_keystroke(keystroke: &str) -> Option<Self> {
        match keystroke {
            "ctrl-o" => Some(Self::CtrlO),
            "ctrl-t" => Some(Self::CtrlT),
            _ => None,
        }
    }
}

/// Whether `bytes` written to the agent carry text, rather than only control keys and escape
/// sequences such as Escape, Ctrl-U, Shift+Tab or a mouse report. Bracketed paste carries text.
pub(crate) fn writes_text(bytes: &[u8]) -> bool {
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        index += 1;
        match byte {
            C0_ESC => match bytes.get(index) {
                // CSI: parameter and intermediate bytes up to a final byte.
                Some(b'[') => {
                    index += 1;
                    while bytes
                        .get(index)
                        .is_some_and(|byte| !(0x40..=0x7e).contains(byte))
                    {
                        index += 1;
                    }
                    index += 1;
                }
                // OSC, DCS, APC and PM strings run to BEL or ST.
                Some(b']' | b'P' | b'_' | b'^') => {
                    index += 1;
                    while let Some(&byte) = bytes.get(index) {
                        index += 1;
                        if byte == 0x07 {
                            break;
                        }
                        if byte == C0_ESC && bytes.get(index) == Some(&b'\\') {
                            index += 1;
                            break;
                        }
                    }
                }
                // SS3 keys carry one more byte.
                Some(b'O') => index += 2,
                // Alt+key.
                Some(_) => index += 1,
                None => {}
            },
            0x00..=0x1f | 0x7f => {}
            _ => return true,
        }
    }
    false
}

const C0_ESC: u8 = 0x1b;

/// Whether text found in the agent's composer can move into Clinch's box. An image is only a
/// placeholder there, so taking its text would drop the image.
pub(crate) fn adoptable_native_draft(draft: &str) -> bool {
    !draft.trim().is_empty() && !draft.contains("[Image #")
}

/// Whether the box's text is a slash command for the agent's own command menu: one line that
/// starts with `/`.
pub(crate) fn is_agent_command(text: &str) -> bool {
    text.starts_with('/') && !text.contains('\n')
}

/// Keys that edit the agent's one-line composer, whose cursor is at the end.
pub(crate) struct ComposerKeys<'a> {
    /// Deletes the character before the cursor.
    pub(crate) backspace: &'a [u8],
    /// Deletes back to the start of the line.
    pub(crate) clear_line: &'a [u8],
}

/// The keys that turn the agent's composer text `from` into `to`: a Backspace for the one
/// character after the text they share, then the rest of `to`. Deleting more clears the line and
/// types all of `to`, since Codex can drop Backspaces that arrive together.
pub(crate) fn composer_edit_bytes(from: &str, to: &str, keys: &ComposerKeys) -> Vec<u8> {
    let shared = from
        .chars()
        .zip(to.chars())
        .take_while(|(old, new)| old == new)
        .count();
    let (mut bytes, kept) = match from.chars().count() - shared {
        0 => (Vec::new(), shared),
        1 => (keys.backspace.to_vec(), shared),
        _ => (keys.clear_line.to_vec(), 0),
    };
    bytes.extend(to.chars().skip(kept).collect::<String>().into_bytes());
    bytes
}

/// Text moving into the box, ahead of what the box already holds.
pub(crate) fn joined_drafts(incoming: &str, existing: &str) -> String {
    if existing.trim().is_empty() {
        incoming.to_owned()
    } else {
        format!("{incoming}\n{existing}")
    }
}

/// Asks a grid element to cover an agent's empty composer so the rich input can replace it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OneBoxComposer {
    pub(crate) agent: CLIAgent,
    pub(crate) terminal_view_id: EntityId,
    /// Also covers text in the agent's composer. A send types into it and presses Enter a moment
    /// later; covering that text keeps the agent's box from flashing up meanwhile.
    pub(crate) cover_drafts: bool,
}

/// The position id of the composer rectangle covered this frame.
pub(crate) fn one_box_composer_position_id(terminal_view_id: EntityId) -> String {
    format!("terminal_view:one_box_composer_{terminal_view_id}")
}

/// Where a grid element draws its rows.
pub(crate) struct GridRows {
    /// Top-left corner of grid row 0, which may be scrolled out of view.
    pub(crate) origin: Vector2F,
    /// The element's visible area.
    pub(crate) bounds: RectF,
    pub(crate) cell_height: f32,
}

impl OneBoxComposer {
    /// Covers the agent's composer with `background` and records its rectangle, when the composer
    /// is visible and empty. Paint this after the grid, cursor and selections.
    pub(crate) fn paint_cover(
        &self,
        grid: &GridHandler,
        rows: GridRows,
        background: Fill,
        ctx: &mut PaintContext,
    ) {
        let Some(area) = native_composer_area(grid, self.agent)
            .filter(|area| self.cover_drafts || area.is_empty())
        else {
            return;
        };
        // A composer scrolled even partly out of view is left alone, so the box is never placed
        // over the footer or outside the pane.
        let Some(rect) = composer_rect(&area.rows, &rows) else {
            return;
        };
        // A layer of its own: glyphs draw after every rect in a layer, so a rect in the grid's layer
        // would sit under the composer's text instead of hiding it.
        ctx.scene.start_layer(ClipBounds::ActiveLayer);
        ctx.scene
            .draw_rect_without_hit_recording(rect)
            .with_background(background);
        ctx.scene.stop_layer();
        ctx.position_cache.cache_position_for_one_frame(
            one_box_composer_position_id(self.terminal_view_id),
            rect,
        );
    }
}

/// The composer's rectangle, when it lies wholly inside the element's visible area.
fn composer_rect(composer_rows: &RangeInclusive<usize>, rows: &GridRows) -> Option<RectF> {
    /// Rounding in row placement, in pixels.
    const TOLERANCE: f32 = 0.5;
    let first_row = *composer_rows.start() as f32;
    let row_count = (composer_rows.end() - composer_rows.start() + 1) as f32;
    let rect = RectF::new(
        vec2f(
            rows.bounds.origin_x(),
            rows.origin.y() + first_row * rows.cell_height,
        ),
        vec2f(rows.bounds.width(), row_count * rows.cell_height),
    );
    (rect.min_y() >= rows.bounds.min_y() - TOLERANCE
        && rect.max_y() <= rows.bounds.max_y() + TOLERANCE)
        .then_some(rect)
}

#[cfg(test)]
#[path = "one_box_composer_tests.rs"]
mod tests;

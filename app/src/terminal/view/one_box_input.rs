//! TerminalView glue for one-box agent input; see [`crate::terminal::one_box_composer`].
//!
//! Visibility follows the screen frame by frame: the box is drawn only in frames where the grid
//! element covered an empty composer. Focus and text follow a little later, through a scan that
//! runs shortly after the screen changes:
//! - Once the agent's own screen has replaced its composer for two scans, the box closes (saving
//!   its text and handing focus to the terminal, where the agent's picker or prompt reads keys),
//!   and it reopens with that text once an empty composer is back.
//! - Text the agent puts in its own composer, such as a message restored by its rewind picker,
//!   moves into the box. It is deleted there with Ctrl-U, which neither agent treats as an
//!   interrupt, and lands in the box only once the composer reads empty, so it is never lost or
//!   shown twice.
//! - Text written into the agent's composer from the keyboard or a send stays there until that
//!   composer is empty again.
//! - A slash command typed into the box is copied into the agent's composer as it is typed, under
//!   the box, so the agent's own command menu lists every command, skill and plugin it has. Up,
//!   Down, Tab and Enter go to that menu, Escape deletes the command, and whatever the agent does
//!   to its composer (completing a command, running it) is read back into the box.
//!
//! Closing the box by hand turns all of this off for the session, as it does for auto-toggle.
use std::time::Duration;

use instant::Instant;
use warp_core::ui::tokens::{RADIUS_MD, SPACE_6};
use warp_terminal::model::escape_sequences::{KeystrokeWithDetails, ToEscapeSequence as _};
use warpui::elements::{
    Align, AnchorPair, Border, Container, CornerRadius, CrossAxisAlignment, Expanded, Fill, Flex,
    OffsetPositioning, OffsetType, ParentElement as _, PositionedElementOffsetBounds,
    PositioningAxis, Stack, Text, XAxisAnchor, YAxisAnchor,
};
use warpui::keymap::Keystroke;
use warpui::r#async::Timer;
use warpui::{AppContext, Element as _, SingletonEntity as _, ViewContext};

use super::use_agent_footer::{
    cli_agent_native_grid, mode_correct_control_key_bytes, native_composer_area,
    native_screen_has_text,
};
use super::TerminalView;
use crate::appearance::Appearance;
use crate::settings::AISettings;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputEntrypoint, CLIAgentRichInputCloseReason, CLIAgentSessionsModel,
};
use crate::terminal::model::escape_sequences::{EscCodes, C0, C1};
use crate::terminal::one_box_composer::{
    adoptable_native_draft, composer_edit_bytes, is_agent_command, joined_drafts, one_box_agent,
    one_box_composer_position_id, supports_one_box, writes_text, ComposerKeys, OneBoxComposer,
    OneBoxKey,
};
use crate::terminal::{CLIAgent, TerminalModel};

/// Keeps the box's border off the pane edges.
const ONE_BOX_HORIZONTAL_MARGIN: f32 = 6.;
/// How long after a screen change the box checks whether to step aside or come back.
const ONE_BOX_SCAN_DELAY: Duration = Duration::from_millis(100);
/// A send writes text into the agent's composer and presses Enter after a short delay. Until the
/// agent has taken it, the cover hides that text and the box doesn't step aside.
const ONE_BOX_SEND_GRACE: Duration = Duration::from_millis(1500);
/// Scans without an empty composer before the box steps aside, so a redraw that drops the
/// composer for a moment doesn't move focus.
const ONE_BOX_STEP_ASIDE_SCANS: u8 = 2;
/// An empty composer this soon after text was written to the agent may not show that text yet.
const ONE_BOX_NATIVE_TEXT_SETTLE: Duration = Duration::from_millis(600);
/// How long the agent gets to delete text that is moving into the box before it is left there.
const ONE_BOX_ADOPT_TIMEOUT: Duration = Duration::from_millis(1500);
/// Two presses of Escape this close together clear the box, as they clear Claude Code's composer.
const ONE_BOX_DOUBLE_ESCAPE: Duration = Duration::from_millis(800);
/// The longest argument hint shown in the box; longer ones end in an ellipsis. In a narrow pane
/// the hint gets at most a third of the width, and none when that is less than
/// [`ONE_BOX_COMMAND_HINT_MIN_CHARS`], so the command keeps its room.
const ONE_BOX_COMMAND_HINT_MAX_CHARS: usize = 60;
const ONE_BOX_COMMAND_HINT_MIN_CHARS: usize = 8;
/// How long the agent gets to draw a slash command's latest keys before the box reads its composer
/// back. Typing faster than this is never overwritten by a composer that hasn't caught up.
const ONE_BOX_COMMAND_SETTLE: Duration = Duration::from_millis(250);

/// A slash command in the box, kept in the agent's composer under it.
struct OneBoxCommand {
    /// What the agent's composer holds: what was copied there, or what the box read back after the
    /// agent changed it.
    mirrored: String,
    /// When the agent will have drawn the latest keys.
    settles_at: Instant,
    /// Tab or Enter went to the agent since the box last changed the command, so the agent may have
    /// changed its composer: completed the command, or run it and emptied the composer. Otherwise
    /// the box's text is what counts, and the composer is not read back; a long command can wrap
    /// there, and a composer that hasn't caught up with fast typing would drop keys.
    agent_may_edit: bool,
    /// The arguments the agent says the command takes, shown at the box's right edge: the box
    /// covers the row where the agent draws them.
    hint: Option<String>,
}

/// One-box bookkeeping for a pane.
#[derive(Default)]
pub(super) struct OneBoxState {
    scan_pending: bool,
    scans_without_composer: u8,
    send_grace_until: Option<Instant>,
    /// When text last went into the agent's own composer from the keyboard or a send. Until that
    /// composer has read empty since, its text belongs there and is left alone.
    native_text_at: Option<Instant>,
    /// Text being moved out of the agent's composer, and when its deletion was sent.
    adopting: Option<(String, Instant)>,
    /// The agent's own screen shows where its empty composer was: a dialog, a picker, its command
    /// menu, or text typed into it.
    pub(super) native_screen: bool,
    /// When Escape last reached the box while it held text.
    last_escape_with_text: Option<Instant>,
    /// The slash command the agent's menu is showing, if any.
    command: Option<OneBoxCommand>,
    /// When the box last deleted its copy of a command from the agent's composer. Until that
    /// composer has redrawn empty, the copy still shows there and is not the agent's to give back.
    withdrawn_at: Option<Instant>,
}

impl OneBoxState {
    fn send_grace_remaining(&self) -> Option<Duration> {
        self.send_grace_until
            .and_then(|until| until.checked_duration_since(Instant::now()))
            .filter(|remaining| !remaining.is_zero())
    }
}

impl TerminalView {
    /// The agent composer Clinch's input box replaces in this pane, if any.
    pub(super) fn one_box_composer(
        &self,
        model: &TerminalModel,
        app: &AppContext,
    ) -> Option<OneBoxComposer> {
        one_box_agent(model, self.view_id, app).map(|agent| OneBoxComposer {
            agent,
            terminal_view_id: self.view_id,
            cover_drafts: self.one_box.send_grace_remaining().is_some()
                || self.one_box.adopting.is_some()
                || self.one_box.command.is_some()
                // The box stays in place, taking keys, while its deleted copy is redrawn away.
                || self.one_box.withdrawn_at.is_some(),
        })
    }

    /// Adds the rich input over the composer rectangle the grid element covered this frame,
    /// growing upward from the composer's bottom edge. The anchor is conditional, so a frame
    /// without a readable empty composer skips the input and the agent's own screen shows.
    /// `columns` is the pane's width in cells.
    pub(super) fn add_one_box_input(&self, stack: &mut Stack, columns: usize, app: &AppContext) {
        let appearance = Appearance::as_ref(app);
        let hint = self
            .one_box
            .command
            .as_ref()
            .and_then(|command| command.hint.as_deref())
            .and_then(|hint| shortened_hint(hint, columns));
        let content = match hint {
            Some(hint) => Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Expanded::new(1., self.render_input()).finish())
                .with_child(
                    Container::new(
                        Text::new_inline(
                            hint,
                            appearance.monospace_font_family(),
                            appearance.monospace_font_size(),
                        )
                        .with_color(appearance.tokens().text_3)
                        .finish(),
                    )
                    .with_margin_right(SPACE_6)
                    .finish(),
                )
                .finish(),
            None => self.render_input(),
        };
        let input = Container::new(content)
            .with_background(Fill::Solid(appearance.tokens().surface_field))
            .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
            .with_corner_radius(CornerRadius::with_all(RADIUS_MD))
            .with_margin_left(ONE_BOX_HORIZONTAL_MARGIN)
            .with_margin_right(ONE_BOX_HORIZONTAL_MARGIN)
            .finish();
        let position_id = one_box_composer_position_id(self.view_id);
        stack.add_positioned_child(
            // The stack hands positioned children its own minimum size, and at layout the
            // composer's one-frame position is not known yet, so the child gets the pane's full
            // height. Align keeps the box at its natural height at the bottom of that space, and
            // unbounded positioning lets that space reach above the pane instead of being pushed
            // down into it, which would drop the box to the bottom of the pane. Sideways it stays
            // inside the pane.
            Align::new(input).bottom_left().finish(),
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_stack_child(
                    &position_id,
                    PositionedElementOffsetBounds::ParentByPosition,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                )
                .with_conditional_anchor(),
                PositioningAxis::relative_to_stack_child(
                    &position_id,
                    PositionedElementOffsetBounds::Unbounded,
                    OffsetType::Pixel(0.),
                    AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Bottom),
                )
                .with_conditional_anchor(),
            ),
        );
    }

    /// Sends `key` to the agent when Clinch's box is replacing its composer. Returns whether the
    /// box took the key.
    ///
    /// Escape with a slash command in the box deletes it. Escape with other text acts on the box
    /// the way the agent's own composer does (see `clear_one_box_on_second_escape`); every other
    /// Escape goes to the agent, interrupting a turn, and twice opening its rewind or transcript.
    pub(super) fn forward_key_to_one_box_agent(
        &mut self,
        key: OneBoxKey,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if one_box_agent(&self.model.lock(), self.view_id, ctx).is_none() {
            return false;
        }
        if let Some(command) = &mut self.one_box.command {
            match key {
                OneBoxKey::Escape => {
                    // Deleting the box's text deletes the copy in the agent's composer as well.
                    self.input.update(ctx, |input, ctx| {
                        input.replace_buffer_content("", ctx);
                    });
                    return true;
                }
                OneBoxKey::Up | OneBoxKey::Down => {}
                OneBoxKey::Tab | OneBoxKey::Enter => {
                    command.agent_may_edit = true;
                    command.settles_at = Instant::now() + ONE_BOX_SCAN_DELAY;
                }
                OneBoxKey::ShiftTab | OneBoxKey::CtrlO | OneBoxKey::CtrlT => {}
            }
        } else if key == OneBoxKey::Escape && self.clear_one_box_on_second_escape(ctx) {
            return true;
        }
        let bytes = {
            let model = self.model.lock();
            match key {
                OneBoxKey::Escape => {
                    mode_correct_control_key_bytes(&model, "escape", "escape", "\u{1b}", C0::ESC)
                }
                // The same sequence the terminal sends for Shift+Tab in a running command.
                OneBoxKey::ShiftTab => {
                    EscCodes::build_escape_sequence_with_c1(C1::CSI, EscCodes::BACKWARD_TABULATION)
                }
                OneBoxKey::CtrlO => {
                    mode_correct_control_key_bytes(&model, "ctrl-o", "o", "\u{f}", 0x0f)
                }
                OneBoxKey::CtrlT => {
                    mode_correct_control_key_bytes(&model, "ctrl-t", "t", "\u{14}", 0x14)
                }
                // Application cursor mode changes the sequence.
                OneBoxKey::Up => arrow_key_bytes(&model, "up", b"\x1b[A"),
                OneBoxKey::Down => arrow_key_bytes(&model, "down", b"\x1b[B"),
                OneBoxKey::Tab => {
                    mode_correct_control_key_bytes(&model, "tab", "tab", "\t", C0::HT)
                }
                OneBoxKey::Enter => {
                    mode_correct_control_key_bytes(&model, "enter", "enter", "\r", C0::CR)
                }
            }
        };
        self.write_user_bytes_to_pty(bytes, ctx);
        if self.one_box.command.is_some() {
            self.schedule_one_box_scan_after(ONE_BOX_SCAN_DELAY, ctx);
        }
        true
    }

    /// Escape with text in the box. A second press soon after the first clears the box; the clear
    /// can be undone. While the agent is known to be idle, the first press stays in the box too, as
    /// it does in the agent's own composer: reaching an idle agent, it would count toward the double
    /// Escape that opens its rewind or transcript. Returns whether the box took the press.
    fn clear_one_box_on_second_escape(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if self.input.as_ref(ctx).buffer_text(ctx).is_empty() {
            self.one_box.last_escape_with_text = None;
            return false;
        }
        let now = Instant::now();
        let is_second = self
            .one_box
            .last_escape_with_text
            .is_some_and(|at| now.duration_since(at) <= ONE_BOX_DOUBLE_ESCAPE);
        if is_second {
            self.one_box.last_escape_with_text = None;
            self.input.update(ctx, |input, ctx| {
                input.replace_buffer_content("", ctx);
            });
            return true;
        }
        self.one_box.last_escape_with_text = Some(now);
        // Without hooks the session can't say whether a turn runs, so the press goes on to
        // interrupt one.
        CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .is_some_and(|session| session.supports_rich_status() && !session.is_actively_working())
    }

    /// Keeps the agent's composer in step with a slash command in the box, so the agent's own
    /// menu lists and runs it. Call after every edit of a slash command in the box. Copying starts
    /// when the box's text becomes a command, and when it stops being one, what was copied is
    /// deleted again.
    pub(super) fn sync_one_box_command(&mut self, ctx: &mut ViewContext<Self>) {
        let text = self.input.as_ref(ctx).buffer_text(ctx);
        let (is_replacing, [backspace, clear_line]) = {
            let model = self.model.lock();
            (
                one_box_agent(&model, self.view_id, ctx).is_some(),
                composer_key_bytes(&model),
            )
        };
        let keys = ComposerKeys {
            backspace: &backspace,
            clear_line: &clear_line,
        };
        if !is_replacing {
            // The agent is gone or no longer under the box, so its composer is left alone.
            self.finish_one_box_command(false, ctx);
            return;
        }
        let target = if is_agent_command(&text) {
            text.as_str()
        } else {
            ""
        };
        let now = Instant::now();
        // The agent's composer must be empty to take a copy: nothing on its way into the box, and
        // nothing typed or sent there. The next edit tries again.
        let composer_is_free = self.one_box.adopting.is_none()
            && self.one_box.native_text_at.is_none()
            && self.one_box.send_grace_remaining().is_none();
        let bytes = match &mut self.one_box.command {
            Some(command) => {
                let bytes = composer_edit_bytes(&command.mirrored, target, &keys);
                target.clone_into(&mut command.mirrored);
                command.settles_at = now + ONE_BOX_COMMAND_SETTLE;
                command.agent_may_edit = false;
                if !bytes.is_empty() {
                    // The agent drops its hint once arguments are typed; look again when it settles.
                    command.hint = None;
                }
                bytes
            }
            None if target.is_empty() => return,
            // The composer still holds a message on its way to the agent. The command is copied
            // once the composer reads empty; see `reconcile_one_box_input`.
            None if !composer_is_free => {
                self.schedule_one_box_scan(ctx);
                return;
            }
            None => {
                self.one_box.command = Some(OneBoxCommand {
                    mirrored: target.to_owned(),
                    settles_at: now + ONE_BOX_COMMAND_SETTLE,
                    agent_may_edit: false,
                    hint: None,
                });
                self.input
                    .update(ctx, |input, ctx| input.set_one_box_command_menu(true, ctx));
                // The cover now hides the command in the agent's composer, and a Coordinator's
                // start screen makes way for the menu.
                ctx.notify();
                target.as_bytes().to_vec()
            }
        };
        // Not a user write: the text is the box's, and the agent only holds a copy.
        if !bytes.is_empty() {
            self.write_to_pty(bytes, ctx);
        }
        if target.is_empty() {
            self.one_box.withdrawn_at = Some(now);
            self.finish_one_box_command(false, ctx);
        } else {
            self.schedule_one_box_scan_after(ONE_BOX_COMMAND_SETTLE, ctx);
        }
    }

    /// Leaves command mode. With `clear_box`, the agent has taken the command, so the box empties.
    fn finish_one_box_command(&mut self, clear_box: bool, ctx: &mut ViewContext<Self>) {
        if self.one_box.command.take().is_none() {
            return;
        }
        self.input.update(ctx, |input, ctx| {
            input.set_one_box_command_menu(false, ctx);
            if clear_box {
                input.clear_buffer_and_reset_undo_stack(ctx);
            }
        });
        ctx.notify();
    }

    /// Call before the box closes. It keeps a slash command it holds, so the copy in the agent's
    /// composer is deleted; reopening copies it again.
    pub(super) fn withdraw_one_box_command(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(command) = &self.one_box.command else {
            return;
        };
        let key_bytes = {
            let model = self.model.lock();
            // An agent that has exited leaves a shell behind, which must not get the keys.
            one_box_agent(&model, self.view_id, ctx).map(|_| composer_key_bytes(&model))
        };
        if let Some([backspace, clear_line]) = key_bytes {
            let keys = ComposerKeys {
                backspace: &backspace,
                clear_line: &clear_line,
            };
            let bytes = composer_edit_bytes(&command.mirrored, "", &keys);
            self.write_to_pty(bytes, ctx);
            self.one_box.withdrawn_at = Some(Instant::now());
        }
        self.finish_one_box_command(false, ctx);
    }

    /// Lets a send's text reach the agent at once, as if its grace period had passed.
    #[cfg(test)]
    pub(super) fn end_one_box_send_grace(&mut self) {
        self.one_box.send_grace_until = None;
        self.one_box.native_text_at = None;
    }

    /// Whether the agent's command menu is showing a slash command from the box.
    pub(super) fn has_one_box_command(&self) -> bool {
        self.one_box.command.is_some()
    }

    /// Notes a user write to the PTY. Text, unlike keys such as Escape or Ctrl-U, lands in the
    /// agent's composer and belongs there until the composer is empty again.
    pub(super) fn note_one_box_pty_write(&mut self, bytes: &[u8]) {
        if writes_text(bytes) {
            self.one_box.native_text_at = Some(Instant::now());
        }
    }

    /// Call when text is about to be typed into the agent's composer and submitted. The text is
    /// the agent's from then on, and a Coordinator's start screen gives way to the conversation.
    pub(super) fn start_one_box_send_grace(&mut self, ctx: &mut ViewContext<Self>) {
        let now = Instant::now();
        self.one_box.send_grace_until = Some(now + ONE_BOX_SEND_GRACE);
        self.one_box.native_text_at = Some(now);
        self.dismiss_coordinator_start_screen(ctx);
    }

    /// Schedules a check of whether the box should step aside or come back. Screen changes call
    /// this; changes in between join one check.
    pub(super) fn schedule_one_box_scan(&mut self, ctx: &mut ViewContext<Self>) {
        self.schedule_one_box_scan_after(ONE_BOX_SCAN_DELAY, ctx);
    }

    fn schedule_one_box_scan_after(&mut self, delay: Duration, ctx: &mut ViewContext<Self>) {
        if self.one_box.scan_pending || self.one_box_session_agent(ctx).is_none() {
            return;
        }
        self.one_box.scan_pending = true;
        ctx.spawn(Timer::after(delay), |me, _, ctx| {
            me.one_box.scan_pending = false;
            me.reconcile_one_box_input(ctx);
        });
    }

    /// The pane's agent when one-box mode manages its rich input.
    pub(super) fn one_box_session_agent(&self, app: &AppContext) -> Option<CLIAgent> {
        if !*AISettings::as_ref(app).one_box_agent_input {
            return None;
        }
        CLIAgentSessionsModel::as_ref(app)
            .session(self.view_id)
            .filter(|session| {
                supports_one_box(session.agent)
                    && !session.is_remote()
                    && session.should_auto_toggle_input
            })
            .map(|session| session.agent)
    }

    pub(super) fn reconcile_one_box_input(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(agent) = self.one_box_session_agent(ctx) else {
            return;
        };
        let (area, screen_has_text, clear_line) = {
            let model = self.model.lock();
            if model
                .block_list()
                .active_block()
                .is_active_and_long_running()
            {
                let grid = cli_agent_native_grid(&model);
                (
                    native_composer_area(grid, agent),
                    native_screen_has_text(grid),
                    clear_line_bytes(&model),
                )
            } else {
                (None, false, Vec::new())
            }
        };
        let now = Instant::now();
        let composer_is_empty = area.as_ref().is_some_and(|area| area.is_empty());
        // The next prompt Claude Code predicts shows as ghost text in the box, as it would in its
        // own composer, and Tab or → accepts it.
        let suggestion = area
            .as_ref()
            .filter(|area| area.is_empty())
            .and_then(|area| area.suggestion())
            .map(str::to_owned);
        self.input.update(ctx, |input, ctx| {
            input.set_agent_suggestion(suggestion, ctx)
        });

        if let Some(command) = &mut self.one_box.command {
            if let Some(wait) = command.settles_at.checked_duration_since(now) {
                self.schedule_one_box_scan_after(wait.max(ONE_BOX_SCAN_DELAY), ctx);
                return;
            }
            let hint = area
                .as_ref()
                .and_then(|area| area.hint())
                .map(str::to_owned);
            if command.hint != hint {
                command.hint = hint;
                ctx.notify();
            }
            match area.as_ref().map(|area| (area.is_empty(), area.draft())) {
                // Enter ran the command.
                Some((true, _)) if command.agent_may_edit => {
                    self.finish_one_box_command(true, ctx);
                    return;
                }
                // Tab or Enter completed the command in the agent's composer; the box shows the
                // completed text.
                Some((false, Some(draft)))
                    if command.agent_may_edit && draft != command.mirrored =>
                {
                    draft.clone_into(&mut command.mirrored);
                    let draft = draft.to_owned();
                    self.input.update(ctx, |input, ctx| {
                        input.replace_buffer_content(&draft, ctx);
                    });
                    return;
                }
                Some(_) => return,
                // Enter opened the agent's own screen, such as a picker, which now has the command.
                // The box steps aside below.
                None if command.agent_may_edit => self.finish_one_box_command(true, ctx),
                // Something else covers the composer, such as a permission dialog. The command
                // stays in the agent's composer underneath; if the box steps aside below, it
                // moves back into the box once the composer shows again.
                None => {}
            }
        }

        // A command the box just deleted from the agent's composer may not be redrawn away yet.
        if let Some(withdrawn_at) = self.one_box.withdrawn_at {
            if composer_is_empty || now.duration_since(withdrawn_at) >= ONE_BOX_ADOPT_TIMEOUT {
                self.one_box.withdrawn_at = None;
            } else {
                self.schedule_one_box_scan(ctx);
                return;
            }
        }

        // Text written into the agent's composer stays there until the composer reads empty a
        // moment after the last of it was written.
        if let Some(written_at) = self.one_box.native_text_at {
            if composer_is_empty && now.duration_since(written_at) >= ONE_BOX_NATIVE_TEXT_SETTLE {
                self.one_box.native_text_at = None;
            }
        }

        if let Some((text, deleted_at)) = self.one_box.adopting.take() {
            if composer_is_empty {
                self.place_adopted_text(text, ctx);
                return;
            }
            if now.duration_since(deleted_at) < ONE_BOX_ADOPT_TIMEOUT {
                self.one_box.adopting = Some((text, deleted_at));
                self.schedule_one_box_scan(ctx);
                return;
            }
            // The agent kept the text, so it stays there rather than showing up twice.
            self.one_box.native_text_at = Some(now);
        }

        let is_open = self.has_active_cli_agent_input_session(ctx);
        if composer_is_empty {
            if let Some(written_at) = self.one_box.native_text_at {
                // The agent may not have drawn what was just written; look again once it has.
                let wait =
                    ONE_BOX_NATIVE_TEXT_SETTLE.saturating_sub(now.duration_since(written_at));
                self.schedule_one_box_scan_after(wait.max(ONE_BOX_SCAN_DELAY), ctx);
                return;
            }
            self.one_box.scans_without_composer = 0;
            if self.one_box.native_screen {
                self.one_box.native_screen = false;
                ctx.notify();
            }
            if !is_open {
                self.open_cli_agent_rich_input(CLIAgentInputEntrypoint::AutoShow, ctx);
            } else if self.one_box.command.is_none()
                && is_agent_command(&self.input.as_ref(ctx).buffer_text(ctx))
            {
                // A command typed while the composer was still taking a sent message.
                self.sync_one_box_command(ctx);
            }
            return;
        }

        if let Some(area) = &area {
            if let Some(draft) = area.draft() {
                // Text Clinch just sent is still in the agent's composer; check once it has gone.
                if let Some(remaining) = self.one_box.send_grace_remaining() {
                    self.schedule_one_box_scan_after(remaining, ctx);
                    return;
                }
                if self.one_box.native_text_at.is_none() && adoptable_native_draft(draft) {
                    // Ctrl-U deletes back to the start of a row and joins an empty row to the one
                    // above, so two per row clear the composer; extras on an empty one do nothing.
                    let presses = 2 * (area.rows.clone().count() + draft.lines().count());
                    self.write_to_pty(clear_line.repeat(presses.min(400)), ctx);
                    self.one_box.adopting = Some((draft.to_owned(), now));
                    self.schedule_one_box_scan(ctx);
                    return;
                }
            }
        }

        // The agent's own screen is showing. An open box steps aside after a second scan agrees;
        // a box that never opened (a dialog at start-up) only records it, once there is a screen.
        if !is_open && (self.one_box.native_screen || !screen_has_text) {
            return;
        }
        self.one_box.scans_without_composer += 1;
        if self.one_box.scans_without_composer < ONE_BOX_STEP_ASIDE_SCANS {
            // A dialog can sit still without further output; check again regardless.
            self.schedule_one_box_scan(ctx);
            return;
        }
        self.one_box.scans_without_composer = 0;
        self.one_box.native_screen = true;
        // Without writing to the agent, whose screen now reads keys.
        self.finish_one_box_command(true, ctx);
        if is_open {
            self.close_cli_agent_rich_input(CLIAgentRichInputCloseReason::AutoToggle, ctx);
        }
        ctx.notify();
    }

    /// Puts text taken from the agent's composer into the box, ahead of anything already there,
    /// and opens the box if it had stepped aside.
    fn place_adopted_text(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        self.one_box.scans_without_composer = 0;
        self.one_box.native_screen = false;
        if self.has_active_cli_agent_input_session(ctx) {
            self.input.update(ctx, |input, ctx| {
                let combined = joined_drafts(&text, &input.buffer_text(ctx));
                input.replace_buffer_content(&combined, ctx);
            });
        } else {
            let view_id = self.view_id;
            CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _| {
                let saved = sessions
                    .session(view_id)
                    .and_then(|session| session.draft_text.clone())
                    .unwrap_or_default();
                sessions.set_draft(view_id, joined_drafts(&text, &saved));
            });
            self.open_cli_agent_rich_input(CLIAgentInputEntrypoint::AutoShow, ctx);
        }
        ctx.notify();
    }
}

/// The bytes for an arrow key, which application cursor mode changes.
fn arrow_key_bytes(model: &TerminalModel, key: &str, fallback: &[u8]) -> Vec<u8> {
    KeystrokeWithDetails {
        keystroke: &Keystroke::parse(key).expect("arrow keys are valid keystrokes"),
        key_without_modifiers: Some(key),
        chars: None,
    }
    .to_escape_sequence(model)
    .unwrap_or_else(|| fallback.to_vec())
}

/// `hint`, cut to fit a pane `columns` cells wide, or `None` when the pane is too narrow for one.
fn shortened_hint(hint: &str, columns: usize) -> Option<String> {
    let max_chars = (columns / 3).min(ONE_BOX_COMMAND_HINT_MAX_CHARS);
    if max_chars < ONE_BOX_COMMAND_HINT_MIN_CHARS {
        return None;
    }
    if hint.chars().count() <= max_chars {
        return Some(hint.to_owned());
    }
    let mut shortened: String = hint.chars().take(max_chars - 1).collect();
    shortened.push('…');
    Some(shortened)
}

/// Backspace and Ctrl-U, in the agent's keyboard mode.
fn composer_key_bytes(model: &TerminalModel) -> [Vec<u8>; 2] {
    [
        mode_correct_control_key_bytes(model, "backspace", "backspace", "\u{7f}", C0::DEL),
        clear_line_bytes(model),
    ]
}

/// Ctrl-U, which deletes back to the start of the line, in the agent's keyboard mode.
fn clear_line_bytes(model: &TerminalModel) -> Vec<u8> {
    mode_correct_control_key_bytes(model, "ctrl-u", "u", "\u{15}", C0::NAK)
}

#[cfg(test)]
#[path = "one_box_input_tests.rs"]
mod tests;

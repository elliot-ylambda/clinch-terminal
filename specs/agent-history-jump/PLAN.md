# Jump to message from the CLI-agent history dropdown

Clicking a prompt row in the message-history dropdown scrolls this pane to where that
prompt is rendered in the terminal and briefly highlights it. Rows whose text is not on
screen are visibly disabled rather than silently inert.

## Verified findings that constrain the design

Each of these was checked in the source or against the live database, not assumed.

1. **History rows have no link to the grid.** `AgentPrompt { timestamp, text }`
   (`app/src/agent_resume.rs:42`) is parsed from the agent's transcript JSONL by
   `read_prompt_history`. Nothing records where a prompt was painted.
2. **Rows are inert by construction.** `sync_cli_agent_message_history_dropdown`
   (`app/src/terminal/view.rs:13816`) builds `MenuItemFields` with no `on_select_action`
   and calls `set_selected_to_none`.
3. **Menu items can carry actions.** `MenuItemFields::with_on_select_action`
   (`app/src/menu.rs:812`); `DropdownItemAction` is blanket-implemented for
   `Action + Clone + PartialEq + 'static` (`app/src/view_components/dropdown.rs:39`). The
   dropdown is a child view of `TerminalView` (`app/src/terminal/view/pane_impl.rs:418`),
   so a `TerminalAction` variant propagates up naturally.
4. **Arbitrary-position scrolling already exists.**
   `ScrollPositionUpdate::ScrollToBlocklistRowIfNotVisible` and
   `ScrollToFindMatchIfNotVisible` (`app/src/terminal/block_list_viewport.rs:223-226`).
   The blocklist-row math is `top_of_block_in_lines(block_index) +
   block.block_section_offset_from_top(section)` (`block_list_viewport.rs:1073-1079`).
5. **Grid search is wrap-transparent.** `RegexDFAs::search` walks
   `grid.grapheme_cursor_from(point, Wrap::All)` (`app/src/terminal/model/find.rs:219`),
   so a soft-wrapped prompt matches as one contiguous string.
6. **Restore repaints conversations, but inconsistently.** `SerializedBlock.stylized_output`
   is replayed by `BlockList::initialize` (`app/src/terminal/model/blocks.rs:688`), but only
   for blocks having both `start_ts` and `completed_ts`, truncated to
   `MAX_SERIALIZED_STYLIZED_OUTPUT_LINES = 5000` (`app/src/terminal/model/block.rs:81`), and
   capped at `MAX_TERMINAL_BLOCKS_TO_PERSIST_PER_SESSION = 100` per pane
   (`app/src/persistence/block_list.rs:19`). Checked against the live DB: a `cx` block holds
   636 KB of real conversation, while `clinch_agent_resume_launch claude` blocks hold ~1.8 KB.
7. **Codex renders prompts as `› <text>` verbatim** — confirmed by decoding block 1402 out of
   the live DB. Claude Code's marker is **not yet verified** (see Task 1).

### The load-bearing conclusion

From (6): a scheme that records grid positions at prompt-submit time is wrong for restored
content — rows renumber on replay and no submit event ever fires for replayed text. A scheme
that assumes restored content exists is wrong when it does not. **Only searching what is
currently painted is correct in both cases.** Everything below follows from that.

---

## Phase 0 — the cheap version, do this first

Before building anything bespoke: make a row click **drive the existing find bar**, pre-filled
with the prompt text, and focus the match.

`TerminalFindModel::run_find(FindOptions, ctx)` (`app/src/terminal/find/model.rs:407`) plus the
existing `scroll_to_match` (`app/src/terminal/view.rs:23339`) already deliver scroll,
highlight, match count, and next/prev cycling. This is roughly a day's work against code that
is already tested in production.

**Cost:** it takes over the find bar, discarding whatever the user had typed.

**Decision gate:** live it for a few days. If the find-bar takeover is not annoying, stop here —
Phase 1 is a large amount of bespoke machinery to avoid one side effect. If it does grate,
build Phase 1 and keep Phase 0's resolution semantics.

Phase 1 below assumes the gate came back "annoying."

---

## Phase 1 — bespoke resolution

### Resolution index

Per pane, `PromptLocationIndex` maps history index to
`Resolved(BlockIndex, GridType, Match)` | `Absent` | `Unknown`.

**Search key.** Take the prompt's first non-empty line, trim, split on whitespace, keep the
first ~8 tokens capped at ~64 graphemes. Regex-escape each token with the same `escape()` the
non-regex find path uses, then join with **`\s*`**.

> `\s*`, not `\s+`. At a soft wrap the grapheme cursor emits **no** separator character between
> the last cell of one row and the first of the next, so `\s+` would fail to match any prompt
> long enough to wrap — which is most of them. `\s*` also absorbs trailing row padding and
> continuation-line indentation. Over-matching (`the\s*quick` hitting `thequick`) is harmless
> here.

Run **case-sensitive**: prompts render verbatim, so it costs nothing and cuts false hits.

Reject keys under 8 non-space characters as too weak to disambiguate — they would match
everywhere. Mark those `Absent`.

**Ordered backward pass.** Resolve the *newest* prompt first, scanning leftwards from the end
of the blocklist with `RegexDFAs::regex_search_leftwards`, then prompt N-1 leftwards from
there, and so on. Each search resumes where the last stopped, so the whole pass costs roughly
one linear grid walk.

> Backward, not forward. After a restart the same prompt can be painted **twice** — once in the
> restored static block, once again if the resumed agent replays its history. Backward
> resolution prefers the most recent rendering, which is the live conversation.

If a prompt is not found from the cursor, retry once across the full range; still nothing means
`Absent` and the cursor stays put. Abort the pass on a hard row budget and mark the remainder
`Unknown` — with backward resolution the remainder is the *oldest* prompts, which are the ones
least likely to still be painted.

**No cross-open caching.** Resolve on each dropdown open; cache only for the lifetime of that
open so clicks are instant. This deletes an entire class of staleness bugs (blocks appended,
evicted, cleared, or replayed between opens) for the price of one grid walk on a
user-initiated action with a natural pause in it.

**`Unknown` rows stay clickable** and re-run the same resolver for that one key with a wider
budget. Same function, different arguments — not a second code path.

### Scope and cost

Search the whole blocklist, bounded by a hard row budget, with early exit once every prompt is
resolved. This is the only approach that handles a conversation split across a restored block
and a live one without new bookkeeping.

Run it synchronously on dropdown open to start, and **measure against a 4.9 MB block** (one
exists in the live DB, id 1053). If it stalls the UI, move it to `BlockList::background_executor`
and let rows start as `Unknown`. Do not pre-optimize this.

### Interaction

- `TerminalAction::JumpToAgentPrompt { index }`, handled in `TerminalView::handle_action`.
- In `sync_cli_agent_message_history_dropdown`: `Resolved` and `Unknown` rows get
  `.with_on_select_action(DropdownAction::select_action_and_close(...))`; `Absent` rows get
  `.with_disabled(true)` and a tooltip explaining why.
- Resolve on dropdown **open**, not in `sync_*` — sync runs on every agent status change.
- **Guard alt screen.** `scroll_to_match` early-returns when `is_alt_screen_active()`
  (`view.rs:23341`); the jump must do the same. There is no blocklist scrollback to jump to.

### Scrolling

`scroll_to_match_if_not_visible` uses find-navigation semantics — minimal scroll, so a backward
jump lands the match at the **bottom** of the viewport with its preceding context above it.
Wrong for this feature: you want the prompt at the top with its response below.

Add `ScrollPositionUpdate::ScrollToBlocklistRowAtTop { block_index, section, buffer_lines }`,
reusing the offset math already in `scroll_to_match_if_not_visible`
(`block_list_viewport.rs:1073-1090`).

**Pin the scroll.** Agent panes follow the bottom while the agent streams; the jump must leave
the scroll position locked or auto-follow will yank it straight back.

### Highlight

Landing in the middle of a wall of agent output tells you nothing without one. Store the range
in a new `jump_highlight: Option<(BlockIndex, GridType, Match)>` on `TerminalView` and render it
through the existing find-match highlight path.

**Deliberately not `TerminalFindModel`** — writing there would clobber the user's find bar,
which is the entire reason Phase 1 exists.

Clear it on the next `scroll()`, on any dispatched action, or after ~1.2 s on an executor timer.

---

## Tasks

1. **Verify Claude Code's scrollback rendering** — the prompt marker, whether continuation
   lines are indented, and whether `claude --resume` repaints prior history. Half an hour with a
   live pane plus a persisted block dump. This gates the tokenizer defaults and confirms the
   double-painting assumption behind backward resolution.
2. **Phase 0** — wire row clicks to `run_find` + `scroll_to_match`. Ship it. Run the decision
   gate.
3. `PromptLocationIndex` and the resolver, as a pure function unit-tested against a synthetic
   blocklist, in the style of `app/src/terminal/find/model/block_list_tests.rs`.
4. Expose a side-effect-free search from `crate::terminal::find` — `run_find_on_block_list` is
   `pub(super)`. Prefer putting the resolver *inside* the find module and exporting only the
   resolved index, keeping the search primitives private.
5. `ScrollToBlocklistRowAtTop` variant, viewport handling, scroll lock.
6. `TerminalAction::JumpToAgentPrompt` and its `handle_action` arm.
7. Menu wiring: actions on resolvable rows, disabled + tooltip on absent ones, resolution on
   open, alt-screen guard.
8. Transient highlight and its clear paths.
9. Tests: resolver unit tests; a `view_tests.rs` test that clicking row N dispatches the action;
   an integration test that the viewport actually moves.

## Non-goals, and code that must not be added

- **No new persistence.** Nothing is written to the DB. The index is in-memory and rebuilt.
- **No `TerminalFindModel` mutation** in Phase 1. The user's find bar stays untouched.
- **No prompt-submit anchoring.** Rejected above — do not add position fields to `AgentPrompt`
  or to the `update_from_event` live-prompt path
  (`app/src/terminal/cli_agent_sessions/mod.rs:1027-1050`). It cannot work across restore, which
  is the case that motivated this feature.

## Code to update, not leave stale

- The comment at `app/src/terminal/view.rs:13890` — *"History rows are informational and should
  not become selected"* — is the design statement this feature reverses. Rewrite it to say rows
  are now actionable but still never become the dropdown's *selected value* (the header
  independently shows the latest prompt), and keep the `set_selected_to_none` call.
- `cli_agent_history_prompt_tooltip` (`view.rs:3050`) must compose the "not in scrollback" note
  with the existing full-prompt tooltip rather than duplicating the builder.
- Disabled rows must be checked against the current chrome: 45% outline opacity and 15 px
  history font from the recent declutter pass.

## Known limits — surface these, do not paper over them

All of these land as `Absent` rows, which is the honest outcome:

- Content beyond 5000 lines per block is dropped on restart; beyond 100 blocks per pane, dropped.
- Live eviction at `max_grid_size_limit` (`app/src/terminal/model/blocks.rs:268`).
- `/clear` and compaction wipe the grid in place — agent grids run
  `FullGridClearBehavior::Clear` (`app/src/terminal/model/grid/grid_handler.rs:333`).
- Prompts from sessions that were never rendered in this pane (bridged claude.ai sessions,
  transcripts first run elsewhere).

## Open risk

A backward scan hits the **live input area first** if the prompt text is still sitting in the
agent's input box or Clinch's rich input. Restricting matches to block *output* grids limits
this, but it is not airtight. Confirm the real behaviour in Task 1 before building Phase 1.

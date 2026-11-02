# Clinch redesign: technical plan

Implements [PRODUCT.md](./PRODUCT.md).
- **Visual reference:** `web/design-prototype` (React, not shipped).
- **Token source of truth until Phase 0 lands:** `web/design-prototype/src/tokens.css`.
- **After Phase 0:** `crates/warp_core/src/ui/tokens.rs`.

## Status

What landed on branch `clinch-redesign`, and how it departs from the phases below. The scale itself
moved after the first native build read as "still kinda looks the same": radii are 8/12/16, the
frame sits at an 8% blend (not 2.5%), the card inset is 10px, and the title bar is 44px.

**Landed**
- Phase 0 in full, except that the rate-limited tab state reads `session_context.stop_reason`, which
  was already on the session model.
- Phase 1: tokens on `UiBuilder`, `ActionButton`, the component library, menus, toasts and both dialog
  styles. `script/check-ui-literals` runs in the gate.
- The radius codemod from Phase 7, moved up to Phase 1 because it is the sharp-edges fix itself.
- Phase 2: frame, terminal and split cards, project pills, Remote Control icon, usage meters, Update pill.
- Phase 3: one-line rows, restoring state, details card delay, section headers, foot links with
  popovers. The options popup and the summary and compact modes are deleted, as is `attention_pulse`.
- Phase 4: floating shell input, blocks without dividers or the failed bar, one-row agent footer.
- Phase 6, partly: one sidebar with General first, card rows, Esc closes, Settings takes no sidebar row,
  Cloud platform gated, plugin and subshell strings renamed.

**Not landed**
- Usage and Remote Control popovers with the new content; Remote Control and the usage panel keep
  their existing click targets and panel layout.
- The gear menu's PRODUCT §12 item list.
- The "! N" button cycles to the next waiting session rather than filtering the list.
- Sidebar settings beyond the details card: density, branch visibility, PR/diff badges.
- Grouped sections in More, and icon-only chips in narrow panes (they show fewer chips instead).
- One banner style, including the plugin chip as a one-time banner (it sits under More for now).
- Phase 5: palette scope pills, Resource Center deletion.
- Phase 6: theme grid and theme chooser deletion, footer-button editor in Settings, Remote Control
  checklist, recorder states, AI-only row gating, the remaining Warp strings.
- Phase 7: Roboto, the remaining `ColorU::new` allowlist, the prototype.
- Merging the duplicate tooltip and dialog implementations. They draw the same now, so this is
  only cleanup.

**Checking native builds:** `test_redesign_screens`, `test_redesign_screens_paper` and
`test_redesign_agent_screens` in `crates/integration` save PNGs from an isolated window and profile.
The agent one uses a stand-in `claude`, so it shows the footer and glyph but no live hook states.

## What exists today

- **Theme:** `WarpTheme` has 8 fields (`crates/warp_core/src/ui/theme/mod.rs:586`).
  - Everything else is derived in `theme/color.rs`: `surface_1/2/3` are the background blended
    toward the foreground by 5/10/15%. `outline()` is the foreground at 10%.
  - Status colors are fixed constants: `ui_*_color`, `Fill::warn/error/success`, and the Clinch
    constants in `app/src/ui_components/mod.rs:25-56` (`CLINCH_DONE_BLUE`, `CLINCH_ATTENTION_AMBER`, …).
  - The status-to-color mapping is duplicated in `workspace/view/vertical_tabs.rs:429` and
    `project_window.rs:276`.
  - `ProjectCliAgentActivity` (`workspace/view.rs:1399`) has no rate-limited state.
- **Fonts:**
  - The UI uses bundled Roboto (`app/src/appearance.rs:309-345`), and it is not a setting.
  - The terminal default is bundled Hack (`app/src/settings/font.rs:11`).
- **Geometry:**
  - About 550 `CornerRadius::with_all(Radius::Pixels(..))` literals (4px ×227, 8px ×167, 6px ×61),
    467 `Border::…` calls, 92 local radius constants and about 711 local padding/spacing constants.
  - No shared scale exists.
  - Borders take layout space (`warpui_core/src/elements/gui/container.rs:244`).
  - Drop shadows exist. There is no tween/animation system, only `repaint_after` timers.
- **Components:** four layers, roughly in order of leverage.
  1. `UiBuilder` (`crates/warp_core/src/ui/builder.rs`): defaults for about 1,000 call sites
     (buttons, switches, inputs, tooltips, checkboxes); base radius 4 and a 1px border at `:80-102`.
  2. `view_components/action_button.rs`: about 10 themes, about 100 files.
  3. `menu.rs` (67 files) and `view_components/dropdown.rs` / `filterable_dropdown.rs`.
  4. Three tooltip implementations and two dialog implementations (`ui_components/dialog.rs`,
     `crates/ui_components/src/dialog.rs`).
- **Hand-built chrome:**
  - `workspace/view/vertical_tabs.rs` (8.8k lines), `project_window.rs`, and `workspace/view.rs` (32k lines).
  - The agent footer is `ai/blocklist/agent_view/agent_input_footer/mod.rs`.
  - Settings helpers are in `settings_view/settings_page.rs`.
- **Settings has two surfaces:** the sidebar-less Clinch page (`settings_view/clinch_page.rs`) and
  the inherited terminal settings with their own nav (`settings_view/mod.rs`, `nav.rs`). It opens
  as a pane in its own tab (`workspace/view.rs:9421`).
- **Icons:** the `Icon` enum (`crates/warp_core/src/ui/icons.rs`) maps to SVGs in
  `app/assets/bundled/svg/`. They render as alpha masks tinted by one color. Stroke SVGs work.

## Design decisions

1. **Tokens are derived, not stored.** `tokens.rs` computes semantic values from the existing 8
   theme fields, so the 22 bundled themes, user YAML themes and Base16 imports keep working without
   new fields or serde changes.
   - Surfaces are background→foreground blends:
     - chrome 2.5%
     - field 6.5%
     - raised 10% (same as today's `surface_2`)
     - hover 6.5%
   - Text tiers keep the contrast-picking of `main/sub/hint_text_color`, then nudge lightness
     until they reach 4.5:1 on the darkest and lightest surface they sit on. Low-contrast
     third-party themes stay readable without editing them.
   - Status colors are fixed dark/light pairs chosen by `inferred_color_scheme()`, because they
     must not collide with a theme's accent (the reason `CLINCH_DONE_BLUE` exists).
2. **Themes with background images, gradients or window transparency** get surfaces as foreground
   overlays with alpha (`fg_overlay_*`) instead of opaque blends, so the image and transparency
   still show through.
3. **Borders are disabled before they are deleted.** Primitives first set border color to
   transparent, which keeps geometry. Border removal and spacing retuning happen per surface in the
   phase that redesigns it, where a 1px shift is reviewed anyway.
4. **Settings keeps its pane mechanics.** It stays a pane in its own tab, so persistence, restore
   and split behavior don't change. The sidebar skips rendering that tab, and the pane gets a close
   button and Esc. This is far less risky than turning Settings into a new workspace-level view.
5. **No feature flag.** A runtime "old look" switch would double every touched code path. Phases
   are ordered so each one leaves the app consistent. Phases 2–4 (the most visible ones) should land
   within one release window.
6. **No animation system.** The working-tab pulse and the details-card delay use existing
   `repaint_after` timers. Menus, toasts and dialogs appear without transitions, which is what the
   prototype does.

## Phases

Each phase is one PR off `clinch/main`. It passes the launch gate (nextest, gate clippy) and gets the
GUI check from PRODUCT.md's state list for the surfaces it touched. Sizes are relative.

### Phase 0: Foundations (M)

- Add `crates/warp_core/src/ui/tokens.rs`:
  - `RADIUS_SM/MD/LG` (6/10/14) and `RADIUS_PILL` (`Radius::Percentage(50.)`).
  - A spacing scale `SPACE_1..10` (2…32).
  - `Tokens::from(&WarpTheme)` exposing surfaces (terminal, chrome, field, raised, hover,
    overlay), `hairline`, `control_track`, `text_1/2/3`, `accent`, `accent_soft`,
    `text_on_accent`, `focus_ring`, `status(StatusKind::{Working, NeedsYou, Done, RateLimited})`
    and its soft variant, and the eight section colors.
  - Exposed as `appearance.tokens()`.
- Move the `CLINCH_*` constants into `tokens.rs`. Point the duplicated status-color mappings in
  `vertical_tabs.rs` and `project_window.rs` at `tokens.status()`, then delete the constants.
- Add `RateLimited` to `ProjectCliAgentActivity`. Set it from the existing rate-limit detection
  that drives auto-continue. Persist nothing new.
  - First confirm that detection is per-session state the sidebar can read. Today it is only
    rendered by the footer.
  - If it lives only in the footer view, move it onto the session model in this phase.
    Don't add a second detector.
- Fonts:
  - Bundle Inter (static TTFs: Regular, Medium, SemiBold, Bold and their italics) in place of
    Roboto in `load_default_ui_font_family`.
  - Bundle JetBrains Mono and make it the terminal default only when `FontName` is unset.
  - Keep Hack bundled for people who chose it explicitly.
  - Keep the Windows Segoe UI override.
  - Raise fixed widths that Inter overflows; start with `menu.rs` `DEFAULT_WIDTH = 186`.
- Retune Ink and Paper in `themes/default_themes.rs` to the prototype values:
  - Ink: background `#0D0E11`, foreground `#D3D6DD`, accent `#C5F06B`.
  - Paper: background `#FFFFFF`, foreground `#2A2C31`, accent `#4F7D0F`.
- Add the stroke icon set as SVGs (one 24px viewBox, `app/assets/bundled/svg/clinch/`) with `Icon`
  variants. This covers the sidebar, footer, menus and settings nav; paths are in
  `web/design-prototype/src/icons.tsx`.
- Tests:
  - `tokens` unit tests assert 4.5:1 contrast for `text_1/2/3` on terminal, chrome, field and
    raised surfaces, plus 3:1 for status colors on chrome, **for every bundled theme**. The
    derivation enforces the floor, so the test guards that enforcement.
  - Snapshot the token values for Ink and Paper.

### Phase 1: Shared primitives (L)

- Point `UiBuilder::base_styles`, the button variants, `switch`, `checkbox`, `segmented_control`,
  `text_input` and `slider` at tokens:
  - Radius MD for buttons and fields, SM for chips.
  - Transparent borders.
  - Switch track `control_track`, on-state `accent`.
- Restyle in the same way:
  - `ActionButton` themes and `ui_components/buttons.rs` (icon buttons: SM radius, hover fill,
    pressed raised).
  - `menu.rs`: overlay surface, LG radius, SM item radius, overlay shadow. Submenus, headers and
    separators restyle too.
  - `dropdown.rs` and `filterable_dropdown.rs`.
  - `dismissible_toast.rs` and the agent toast stack: one style, top right under the title bar.
  - Workspace and terminal banners: one banner style for info, warning and error.
- **Unify duplicates:**
  - Three tooltip implementations become one (`crates/ui_components` tooltip). Delete the other
    two and migrate their call sites.
  - Two dialog styles become one (`crates/ui_components` dialog). Delete `ui_components/dialog.rs`
    after migrating its 21 files.
- Add `script/check-ui-literals`, run by `launch-check` (syntax stage).
  - It fails on new `CornerRadius::with_all(Radius::Pixels(` or `ColorU::new(` in `app/src`
    outside an allowlist that shrinks every phase. The allowlist starts as today's files.
  - Its regression test lives in `script/tests/`.

### Phase 2: Window chrome (M)

- `workspace/view.rs` header (`:23463`, `:22555`) and `project_window.rs`:
  - Project pills: raised active fill, no accent border, amber waiting count. The hover card
    restyles to the overlay style.
  - Header toolbar buttons: icon buttons with pressed state.
  - Gear menu becomes the Clinch menu, with items per PRODUCT §12.
- Usage widget (`ai/blocklist/usage/cli_agent_usage_header.rs`): compact bars with 85%/100%
  thresholds. The detail panel (`cli_agent_usage_chip.rs`) becomes the usage popover.
- Remote Control button (`view.rs:23854`): an icon with connected/approve states and a popover.
  Pairing approval moves into the popover; the full QR flow stays in Settings.
- Update pill: quiet style. Its menu folds into the Clinch menu.
- **Terminal card:**
  - The pane group container gets an 8px inset and LG radius on the terminal surface.
  - Split dividers (`pane_group/tree.rs:1427,1463`) become 8px gaps between cards.
  - Inactive panes keep their dim (`pane_group/pane/view/mod.rs:403`).

### Phase 3: Sidebar (L)

All of this is in `workspace/view/vertical_tabs.rs` and `tab.rs`.

- **Rows:**
  - Rows become one 32px line: glyph tinted by `tokens.status()`, amber "!" for needs-you, and a
    dashed ring while restoring.
  - Branch shows only on the selected row, per the new setting.
  - Delete `TabCardState`, card rendering, card borders and the pulsing amber border (`:486`).
- **Details card:** reuse the existing details popout (`:8442`) with a 650ms hover delay and the
  new content order (title, status, branch, folder, PR/diff, last prompt).
- **Section headers** (`:4109`): dot, name, count, and the collapsed needs count.
  - Delete the collapsed icon collage, tinted section boxes and section pulse.
  - The drop target becomes an accent tint.
- **Control bar** (`:1911`): search, the "! N" needs filter (replacing "N waiting") and "+".
  - Delete the view-options gear popup (`:2095`, `:7110`). Its options become Settings →
    Appearance → Sidebar rows.
  - Delete the auto-worktree toggle (`:2172`). It is already in Settings and gains a "+" menu entry.
- **Bookmarks** (`:2534`) **and Tasks** (`:2760`) become two footer links with popovers.
  - Delete the always-open panels and the "Create new section" button (`:1979`).
  - "New section…" moves into "+".
- **Menus:**
  - The tab menu (`tab.rs:243`) gets Move to section / Move to project submenus and the
    PRODUCT §22 order.
  - The multi-select menu gains bulk move, bookmark and close.
- **Blink:** stop using `attention_pulse.rs` in the sidebar. Delete it if no other caller remains
  (title glyphs use `title_glyph.rs`, which stays).
- **Settings → Appearance → Sidebar rows**, added to today's Appearance page (Phase 6 restyles it):
  - Density comfortable/compact. Compact is the existing compact variant (`:8603`), restyled;
    delete the summary variant (`:6068`) if nothing else uses it.
  - Branch visibility.
  - Details card on/off.
  - PR/diff badges.

### Phase 4: Pane and agent chrome (M)

- **History bar:**
  - Becomes the "Last prompt" bar (`terminal/view/pane_impl.rs:400`). It opens the existing
    history dropdown restyled as an overlay; jump-to-message is unchanged.
  - Delete the sticky prompt strip it replaces.
- **Agent footer** (`agent_input_footer/mod.rs:1892`):
  - Status label.
  - The first six toolbelt items as chips, the rest in a grouped **More** menu.
  - Auto-continue switch with contextual label.
  - Width breakpoints at 820px and 520px, measured from the pane width already available at layout.
  - Plugin chips (`:573`) move to a one-time banner; "Update Warp plugin" is renamed.
  - Restyle the footer editor modal (`agent_input_footer/editor.rs`) and quick-insert modal
    through the unified dialog. They move into Settings in Phase 6, once Settings → Agents exists.
- **Plain terminal footer:** same chip style.
- **Blocks** (`terminal/block_list_element.rs:2453`): failed tint without the left bar; selected
  block field surface; hover surface.
- **Inputs, find and pane headers:**
  - Shell input: rounded field surface with context chips.
  - Find bar (`view_components/find.rs:539`): floating overlay style, Aa and regex toggles kept.
  - Split pane headers restyled.

### Phase 5: Palettes, panels and overlays (M)

- **Palettes:**
  - Command palette (`search/command_palette/view.rs`, `search_bar.rs`): scopes as pill toggles,
    overlay style, rows with glyph and subtitle.
  - Conversation finder filters (`agent_conversations/data_source.rs:35`) rendered as two pill groups.
  - The Ctrl-Tab switcher gets the same row style.
- **Left panels:** file explorer, global search and skills restyle as rounded panels beside the card.
- **Resource Center:** "Keyboard shortcuts" now opens Settings → Keyboard shortcuts, which leaves
  the Resource Center unreachable. Delete `resource_center/` including the Warp Essentials page.
- **Remaining modals** restyle through the unified dialog: tab config, worktree config, header
  toolbar editor, theme creator, add regex.

### Phase 6: Settings (L)

- **Merge** the Clinch page into the main settings nav:
  - Its sections move to General, Agents, Remote Control and Computer use.
  - Delete the sidebar-less Clinch page mode (`settings_view/mod.rs:2671`) and the special-cased
    "Clinch Settings" header.
  - Rename Features to Terminal, and Warpify to Subshells and SSH.
- **Restyle the shared helpers** in `settings_page.rs`: group cards (chrome surface, LG radius,
  hairline row separators), 56px rows, the page title and lede, and the search-hit accent outline.
  - Collapse the duplicated `render_group` helpers (`appearance_page.rs:2596`,
    `features_page.rs:4457`) into `settings_page.rs`.
- **Special widgets:**
  - The theme grid replaces the theme preview cards and the theme chooser left panel
    (`themes/theme_chooser.rs`). Delete the panel and point "Switch theme…" at the page. The
    theme creator modal stays.
  - Keybinding table: recorder states.
  - Directory colors.
  - Remote Control checklist (`clinch_page.rs:468-1355`), every state.
  - Footer-button editor: Settings → Agents → Footer buttons replaces the footer editor and
    quick-insert modals. Delete both modals and point "Edit buttons…" at the page.
- **Settings pane:**
  - The sidebar skips the Settings tab.
  - The pane gets a close button and Esc.
  - The gear menu, ⌘, and the old "Clinch Settings…" / "Terminal Settings…" entry points all open
    the right page.
- **Gate or delete:**
  - Gate Cloud platform → Environments behind `has_backend()` (`environments_page.rs:2025`).
  - Hide the "Agent font" row and AI-only Code rows when `!is_any_ai_enabled()`.
  - Rename the remaining Warp strings: notifications copy, "Warp" input mode, the Markdown viewer
    name, and docs.warp.dev links (they point to Clinch docs or are removed).

### Phase 7: Cleanup (S)

- Codemod the remaining radius literals to tokens with a reviewed mapping:
  - 2–5px → SM, 6–8px → MD, 10px and up → LG.
  - Sub-6px indicators, meters and scrollbar thumbs → PILL.
  - The literal allowlist in `check-ui-literals` reaches zero for radii.
- Delete the 92 local `*RADIUS*` constants and the now-unused border constants.
- Delete the Roboto font assets and their loader branch.
- Delete `web/design-prototype` once every PRODUCT state is checked in the native build.
  `tokens.rs` and the specs remain the reference.
- Update `README.md` screenshots and the clinch.sh site screenshots after the final release.

## Code this plan makes dead (and where it is removed)

| Code | Removed in |
|---|---|
| `CLINCH_*` color constants, duplicated status-color mappings | Phase 0 |
| Two of three tooltip implementations, `ui_components/dialog.rs` | Phase 1 |
| Tab cards, card pulse, section boxes and collage, view-options popup, auto-worktree toggle, always-open Bookmarks/Tasks panels, "Create new section" button, sidebar `attention_pulse` use | Phase 3 |
| Sticky prompt strip, footer plugin chips | Phase 4 |
| Resource Center and Warp Essentials | Phase 5 |
| Clinch page mode, duplicated `render_group`, footer editor and quick-insert modals, theme chooser panel, Environments page in Clinch | Phase 6 |
| Radius/border literals and constants, Roboto, the prototype | Phase 7 |

## Verification

- **Every phase:**
  - `cargo nextest run` for touched crates, plus the gate clippy command.
  - The token contrast tests (Phase 0 onward).
  - `check-ui-literals` (Phase 1 onward).
- **GUI:** check every PRODUCT state the phase touched, in Ink and Paper, on a release candidate
  (`make candidate`) or an installed release. Never launch a dev build from a worktree; that is a
  known session-restore hazard. Compare side by side with the prototype at the same window size.
- **Themes:** spot-check three bundled themes with gradients, background images or transparency
  after Phases 1 and 2 to confirm overlays still show the image through.

## Risks

- **Inter is wider than Roboto.** Fixed widths will truncate. Phase 0 includes a sweep of
  fixed-width containers (menus, dropdowns, tooltips, settings labels).
- **Layout shifts** from border removal and the 8px card inset change the terminal grid size by a
  few columns. This is acceptable, but check full-screen apps (vim, htop) after Phase 2.
- **`workspace/view.rs` and `vertical_tabs.rs` are huge and hot.** Other sessions edit them often.
  Keep Phase 3 rebased daily, and merge it promptly once green.
- **Custom themes with unusual palettes** (very low contrast). Tokens keep `font_color()` contrast
  picking for text, so text stays readable even if surfaces look flat.

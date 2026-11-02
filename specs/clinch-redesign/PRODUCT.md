# Clinch redesign: Soft Graphite

## Summary

Clinch's chrome is rebuilt around one rule: **calm by default, details on demand**. Every surface
uses one set of design tokens (three corner radii, a surface scale, three text tiers, one accent,
four status colors). Panels are separated by shade instead of outlines, so the app stops looking
boxy. The vertical tab list shows one mark and one line per tab; everything else appears when you
pause on a tab. Settings becomes one place with one sidebar.

Nothing about how Clinch works changes: same features, same shortcuts, same session restore. Text
inside a terminal (the shell, Claude Code, Codex) is drawn by those programs; Clinch changes only
its font and colors.

The clickable prototype in `web/design-prototype` is the visual reference for every state below.
Run it with `corepack yarn dev` in that folder and open http://127.0.0.1:5317/?proto, which shows the
States panel for jumping to each state.

## Behavior

### Look

1. Interface text uses Inter. The terminal defaults to JetBrains Mono for anyone who never chose a
   font. A font someone chose explicitly is kept.
2. Corners come from one scale: 6px for chips, small buttons and menu items; 10px for tab rows,
   fields, pills and buttons; 14px for panels, menus, dialogs and the terminal card. Dots and meters
   are fully round.
3. Surfaces are separated by shade, never by outlines. The window frame, sidebar and title bar share
   one chrome color. Terminal content sits in a rounded card inset 8px from the frame. Split panes
   are separate cards with an 8px gap instead of a 1px divider.
4. One accent (lime in Ink) is used for the cursor, focus rings, primary buttons, on-switches and
   the update dot. Status uses its own colors: green working, amber needs you, blue done, red rate
   limited. "Needs you" is also a different shape (a filled "!"), so it never depends on color alone.
5. Ink (dark) and Paper (light) are retuned to these tokens. Every other bundled or custom theme
   derives its surfaces from its own background and foreground, so all themes keep working.
6. All text meets 4.5:1 contrast on the surface it sits on, in every bundled theme.

### Title bar

7. Left: sidebar, file explorer, skills and reopen-conversation buttons. Each has a tooltip
   with its shortcut and a pressed state when its panel is open.
8. Project pills: the active project sits on a raised fill with no border. A project with sessions
   that need you shows an amber count. Hovering a pill shows its agent summary card, as today.
9. Plan usage shows as two small bars (Claude, Codex) with percentages. A bar turns amber
   at 85% and red at 100%. Clicking opens a usage popover with each limit, its reset time and Refresh.
10. Remote Control is an icon that turns green when a phone is connected. Clicking opens a
    popover with the device and a link to its settings. Pairing approval requests also surface
    there.
11. "Update" is a quiet pill with an accent dot, shown only when an update is ready.
12. The gear opens the Clinch menu: Settings, Keyboard shortcuts, What's new, Update and
    restart (when ready), Report an issue, Open logs.

### Sidebar

13. Top row: a search field (⌘⇧F), an amber "! N" button that shows only sessions needing you
    (hidden when none do), and "+".
14. "+" offers new Claude Code session, new Codex session, new terminal tab, new section and
    reopen conversation.
15. The pinned Coordinator is always first.
16. Each tab is one 32px line with a mark and a title:
    - The mark is the provider glyph (Claude, Codex, or terminal), tinted by status:
      gray idle, green working, blue done/unread, red rate limited.
    - A tab that needs you shows an amber "!" instead of its glyph.
    - A restoring tab shows a dashed ring and a dimmed title.
    - Idle titles are dimmer than active ones. Titles never switch to bold.
17. Only the selected tab shows a second line: its branch, or its folder for a terminal tab.
    The selected tab sits on a raised fill.
18. Pausing on a tab for about two-thirds of a second shows a details card:
    title, provider and status, branch, folder, pull request with diff stats, and last prompt.
19. Forked and transferred tabs are indented under their source. There is no connector line.
20. Section headers show a small color dot, the name, and a muted count.
    - A collapsed section shows an amber count only when something inside needs you.
    - Hovering shows "+" and "⋯".
    - A section that is a drop target gets an accent tint and outline.
21. Drag shows an accent insertion line. ⌘-click and ⇧-click add tabs to a selection (tinted).
    The right-click menu then acts on all of them: move to section, bookmark, close.
22. The tab right-click menu:
    - Rename, New tab below, Fork in new tab, Transfer to the other agent, Bookmark.
    - Move to section ▸ and Move to project ▸ as submenus.
    - Pin to top, Split right.
    - Close tab last, in red.
    - The Coordinator's menu offers switching provider, Rename and Close coordinator.
23. Tasks and Bookmarks become two quiet links at the bottom of the sidebar. Each opens a
    popover. The task list shows an "Add a task" field and a "start in Claude/Codex" action per task.
24. Empty states: "No sessions yet" with a New session button; "No matching sessions" with
    Clear filters; "Nothing needs you right now" when the needs filter is on and empty.
25. The sidebar no longer blinks. The amber "!" is static. The window-title glyphs and desktop
    notifications still announce attention.
26. Removed from the sidebar, with their settings now in Settings → Appearance → Sidebar:
    - the view-options popup;
    - the auto-worktree toggle (now in Settings → General, with a "+" menu entry);
    - outlined tab cards and section boxes;
    - the always-open Bookmarks and Tasks panels;
    - the "Create new section" button.
    The new settings include row density, branch visibility, the details card, and PR/diff badges.

### Terminal pane

27. Agent tabs show a "Last prompt" bar at the top of the card. Clicking it lists recent prompts;
    clicking one scrolls to it (the existing jump-to-message behavior).
28. The agent footer is one row:
    - a status label (Working · time, Turn complete, Claude/Codex is asking a question,
      Rate limited · resumes in N);
    - the first six footer buttons;
    - a **More** menu holding the rest, grouped as Session, Workflow and Remote, plus "Edit buttons…";
    - the auto-continue switch on the right.

    When rate limited, the switch reads "Continue when it resets", or "Continues in N" once on.
29. In panes narrower than about 820px, footer buttons drop their labels (tooltips keep them).
    Below about 520px, the status label shrinks to its dot.
30. Plain terminal tabs have the same footer style with their own buttons (Claude, Codex, Resume,
    Open in editor, Commit).
31. Blocks:
    - Shell blocks group command and output on a hover-able surface.
    - The selected block uses the field shade.
    - A failed block gets a soft red tint and "Exit N". There is no red left bar.
32. The shell input sits in a rounded field with folder and branch chips.
33. Find (⌘F) is a floating bar with match case, regex, a match count, previous/next and close.
34. Split panes show a slim header with the title and close. The unfocused pane is dimmed.
35. Banners (settings.toml error, restored sessions, plugin notices) are one rounded strip at the
    top of the card, with one action and a dismiss button.
    - The "Enable notifications" plugin chip in the footer becomes a one-time banner.
    - "Update Warp plugin" is renamed "Update Clinch plugin".

### Palettes, panels and overlays

36. The command palette (⌘P) searches tabs, commands and closed conversations, with All / Tabs /
    Commands / Conversations scopes. The conversation finder is the same palette opened on
    Conversations, with This project / All projects / Bookmarked and Any agent / Claude / Codex
    filters.
37. Menus, popovers and dialogs share one overlay style: raised surface, 14px corners, soft shadow,
    6px item corners. Keyboard navigation works in every menu, and → opens a submenu.
38. Notifications appear at the top right under the title bar, at most three at a time, each with
    up to two actions.
39. File explorer, global search and skills open as rounded panels beside the terminal card.
40. Quit, close-tab and install-update confirmations remain native macOS alerts. Only their wording
    is reviewed.
41. "Keyboard shortcuts" opens Settings → Keyboard shortcuts.
    - The Warp-branded "Warp Essentials" tips page is gone.
    - The theme chooser side panel is replaced by the theme grid in Settings → Appearance.
      "Switch theme…" in the palette opens it.

### Settings

42. Settings is one view with one sidebar and one search box covering every page.
    - It replaces the terminal area and closes with × or Esc.
    - It no longer appears as a tab in the sidebar.
43. Pages:
    - **General:** worktrees, startup shell, new-tab folder, Clinch CLI, updates.
    - **Agents:** session capture, tab status, auto-continue default, coordinator, footer buttons, plan usage.
    - **Remote Control.**
    - **Appearance:** theme grid, text, window, cursor, input, sidebar, title bar, tab colors by folder.
    - **Terminal** (formerly Features).
    - **Keyboard shortcuts.**
    - **Subshells and SSH** (formerly Warpify).
    - **Code.**
    - **Privacy.**
    - **Computer use.**
    - **About.**
44. Rows sit in rounded group cards. Each row has a label and optional help text on the left, and
    one control on the right: switch, select, segmented control, slider, number field, or button.
    A search hit gets an accent outline.
45. Remote Control is a three-step checklist (turn on, Tailscale, pair a phone). Its states are:
    off, waiting for Tailscale, showing a QR code, approving a device, paired (with device list),
    and error (with Retry).
46. Footer buttons are edited in Settings → Agents.
    - The first six show in the footer and the rest under More.
    - Rows can be dragged to reorder.
47. Removed: Cloud platform → Environments (requires a Warp login), the "Agent font" row (AI is
    off), and AI-only indexing rows. Every remaining "Warp" string is renamed for Clinch, except
    "Based on the Warp terminal" in About.

## States to check

Every state below exists in the prototype's States panel and must be checked in the native build
before a phase ships. Check each one in both Ink and Paper.

- **Tabs**
  - Tab rows: idle, working, needs you, done, rate limited, restoring.
  - Row interactions: selected, hover with ⋯, hover details card, renaming, multi-selected,
    dragging, drop before/after.
  - Row kinds: forked child, coordinator, terminal.
- **Sections:** expanded, collapsed, collapsed with needs count, hover actions, drop target.
- **Sidebar empty states:** no sessions, no matches, nothing needs you. Also check with the sidebar hidden.
- **Title bar:**
  - active, inactive and waiting pills;
  - usage normal, warning and max;
  - Remote off and connected;
  - update shown and hidden;
  - the gear menu.
- **Agent footer:**
  - every status;
  - More open;
  - auto-continue off, on, and on while rate limited;
  - narrow and very narrow panes.
- **Terminal:** blocks (normal, selected, failed, running), shell input, find, split panes, every banner.
- **Overlays:**
  - palette, every scope, the finder filters, an empty result;
  - tab, section, "+" and gear menus, plus submenus;
  - new section dialog;
  - close coordinator confirmation;
  - toasts (finished, needs you, update ready).
- **Settings:**
  - every page;
  - search with and without hits;
  - every Remote Control state;
  - keybinding recording;
  - theme grid;
  - footer-button editor.

# Project tabs

## Summary

Clinch groups the full workspace that a window represents today into a **project** and lets users keep multiple projects in one OS window as Chrome-style tabs in the top header. Each project keeps its own inner tabs, panes, panels, repository context, and live agent sessions; users can switch projects quickly, see which projects need attention, and drag a project into or out of a window.

## Problem

Working on several repositories currently requires several Clinch windows. That separates related Claude Code and Codex sessions across the desktop and makes it slow to find the project that needs attention. Project tabs put those window-sized workspaces into one navigable header without mixing them into the vertical inner-tab layout.

## Goals

- Let one Clinch window contain several independent, live project workspaces.
- Keep project navigation visually and behaviorally distinct from the existing inner tabs.
- Make repository identity and unread agent activity visible at the project level.
- Preserve the ability to turn any project into its own window and to attach it to another window.

## Non-goals

- Replacing inner tabs, panes, tab groups, or the vertical-tabs panel.
- Treating a Git repository as a persisted database object that must exist before a project can be created; a project may begin empty and acquire repository context later.
- Automatically merging two projects just because their active sessions resolve to the same repository.
- Adding project folders, nested project groups, or manually renamed projects in the first version.

## Figma

Figma: none provided. The interaction should follow the familiar behavior of Chrome's top-level tabs while using Clinch's existing theme, typography, spacing, and window controls.

## Behavior

1. A **project** contains the complete workspace state that one Clinch window contains today: its inner tabs and tab groups, pane layouts, active inner tab and pane, panel visibility and sizing, working-directory context, and running local or agent sessions.

2. Every normal Clinch window contains at least one project. Existing windows and restored sessions created before this feature migrate as windows containing one project, with no loss or reordering of their existing inner tabs or panes.

3. Projects render as a dedicated horizontal strip in the top window header. Project tabs never render in the vertical tabs panel and are never interleaved with inner tabs.

4. In vertical-inner-tab mode, the project strip is the top header and the existing vertical panel continues to list only the active project's inner tabs. In horizontal-inner-tab mode, the project strip remains the top header and the active project's inner-tab strip renders as a distinct row immediately beneath it.

5. The active project is visually distinct using Clinch's existing active/inactive surface and text tokens. Inactive projects remain visible and selectable. The project strip follows the existing window-header visibility behavior in fullscreen or zen modes.

6. Clicking an inactive project tab activates it without recreating its contents. The newly active project restores exactly the inner tab, pane focus, panels, scroll positions, inputs, and transient session state it had when last active.

7. Inactive projects remain live. Terminal processes, Claude Code sessions, Codex sessions, and other agents continue running and may produce notifications while another project is active.

8. A project tab's label is the basename of the repository root associated with that project's active inner tab. The label updates when the user activates a different inner tab whose active session belongs to a different repository, or when that active session moves into a different repository.

9. If the active inner tab has no resolvable repository, the project keeps the most recently resolved repository label from one of its previously active inner tabs. A newly created project with no repository history is labeled `New Project` until repository context is resolved.

10. Repository labels are presentation only: changing a label because the active inner tab changes does not merge, split, reorder, or otherwise alter projects.

11. Long project labels truncate with an ellipsis while retaining the repository name in a tooltip and accessibility label. Duplicate repository labels are allowed; their project order and contents remain distinct.

12. A project tab shows a small notification dot whenever any inner terminal or agent session in that project has unread activity in Clinch's notification model. The dot uses the existing accent/notification token and does not replace or duplicate the detailed notification mailbox.

13. Activating a project alone does not clear its notification dot. The dot disappears only when the underlying notifications are read, removed, or otherwise cease to be unread according to the existing notification behavior.

14. Clicking a desktop or in-app notification for a session in an inactive project first activates the containing window and project, then activates and focuses the originating inner tab and pane. It must not navigate to a similarly positioned pane in the currently active project.

15. Pressing `Command+N` while a normal Clinch window is active creates and activates a new project at the end of that window's project strip. The new project starts with the same empty/default workspace that a new Clinch window starts with today.

16. Pressing `Command+N` when no normal Clinch window is available creates a normal window containing one new project. Quake-mode, onboarding, transient drag-preview, and other special-purpose windows are not used as project containers.

17. The application menu labels the `Command+N` action as **New Project**. An explicit **New Window** command remains available for users and system integrations that require a separate OS window; it creates one window containing one project.

18. `Command+}` activates the next project and `Command+{` activates the previous project. Navigation wraps from the last project to the first and from the first to the last. With only one project, both commands are no-ops.

19. Project switching shortcuts are separate from and do not change the existing inner-tab shortcuts (`Shift+Command+}` and `Shift+Command+{`) or pane-navigation shortcuts. Project shortcuts are discoverable in menus/keybinding settings and may be customized through the existing keybinding system.

20. Project tabs can be reordered within a window by dragging horizontally. Reordering preserves the active project, all project contents, unread state, and most-recently-used state.

21. Dragging a project tab away from the project strip creates a floating preview that follows the pointer. Dropping it on empty desktop space detaches that same live project into a new normal Clinch window at the drop location; sessions are transferred, not restarted or restored from a snapshot.

22. Dragging a project tab over another normal Clinch window's project strip shows an insertion indicator. Dropping attaches the same live project at that position in the target window and focuses it. A project cannot be dropped into a quake-mode, onboarding, transient preview, or otherwise incompatible window.

23. If the dragged project is the source window's only project, dragging on empty desktop space moves that window rather than creating an empty source window. Dropping it into another compatible window closes the now-empty source window after the live project has attached successfully.

24. Cancelling a project drag with Escape or returning it to its source strip leaves the project in the source window at a valid position. A failed transfer also returns the project to the source without losing sessions, tabs, panes, or unread state.

25. A project tab exposes a close button using the same hover and active-state conventions as existing tabs. Closing a project closes all of its inner tabs as one operation and uses the existing running-session/close-confirmation protections before destructive teardown.

26. Closing the active project activates the nearest surviving project: prefer the project that shifts into the closed position, otherwise the previous project. Closing the only project closes the OS window through the existing window-close flow.

27. `Command+W` continues to close the active inner tab/session, and `Shift+Command+W` continues to close the window. Neither shortcut is silently repurposed to close a project in the first version.

28. When project tabs exceed the available header width, tabs shrink to a usable minimum and then become horizontally scrollable or use Clinch's standard overflow affordance. The active project is always scrolled into view, and toolbar/window controls never become inaccessible.

29. Project tab interactions are keyboard and accessibility navigable. Each tab exposes its label, selected state, position in the project list, unread state, close action, and drag/reorder semantics to assistive technologies; focus remains in the active project's content after mouse or shortcut switching unless the user explicitly keyboard-navigates the tab strip.

30. Window-level visual state follows the active project where it is currently derived from the active workspace, including the repository-based header tint and OS window title. Switching projects must not allow a background project to overwrite those active-window values.

31. Session restoration persists physical windows, the ordered projects in each window, the active project, and every project's existing workspace snapshot. After restart, each project returns to the same containing window and order, with the same active inner tab and persisted workspace state.

32. Saving or restoring during a project drag never persists a source/preview duplicate. Persistence records either the last committed source arrangement or the completed target arrangement.

33. Existing actions that explicitly target the active window continue to operate on the active project unless they explicitly request a new project or new window. Actions targeting a pane or terminal by identity locate and activate its containing project before performing the action.

34. Special windows that do not represent a normal terminal workspace—quake mode, authentication/onboarding, web-only simplified views, and transient transfer previews—retain their current behavior unless explicitly made project-compatible later.

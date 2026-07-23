# Project tabs

## Summary

Clinch groups the full workspace that a window represents today into a **project** and lets users keep multiple projects in one OS window as Chrome-style tabs in the top header. Each project keeps its own inner tabs, panes, panels, repository context, and live agent sessions; users can switch projects quickly, see which projects need attention, and drag a project into or out of a window.

## Problem

Working on several repositories currently requires several Clinch windows. That separates related Claude Code and Codex sessions across the desktop and makes it slow to find the project that needs attention. Project tabs put those window-sized workspaces into one navigable header without mixing them into the vertical inner-tab layout.

## Goals

- Let one Clinch window contain several independent, live project workspaces.
- Keep project navigation visually and behaviorally distinct from the existing inner tabs.
- Make repository identity plus live and unread agent/terminal activity visible at the project level.
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

4. Clinch's existing vertical panel continues to list only the active project's inner tabs. The project strip remains a separate horizontal surface in the top header and does not change the fixed vertical-inner-tab layout.

5. The active project is visually distinct with an outline matching the Clinch logo green (`#BFFF00`), while its surface and text continue to use Clinch's existing active/inactive tokens. Inactive projects remain visible and selectable. The project strip follows the existing window-header visibility behavior in fullscreen or zen modes.

6. Clicking an inactive project tab activates it without recreating its contents. The newly active project restores exactly the inner tab, pane focus, panels, scroll positions, inputs, and transient session state it had when last active.

7. Inactive projects remain live. Terminal processes, Claude Code sessions, Codex sessions, and other agents continue running and may produce notifications while another project is active.

8. A project tab's label is the same project-directory name formerly shown above the vertical inner-tab list: the basename of the active inner tab's repository root when detected, otherwise the basename of its current local working directory. The label updates when the user activates a different inner tab or when the active session changes repository or working directory. The vertical inner-tab list no longer renders a separate folder header.

9. If the active inner tab has no resolvable local project directory, the project is labeled `New Project` until path context is resolved.

10. Repository labels are presentation only: changing a label because the active inner tab changes does not merge, split, reorder, or otherwise alter projects.

11. Long project labels truncate with an ellipsis. The complete ordered project list remains available to accessibility output with each project label, position, selected state, and unread state. Duplicate labels are allowed; their project order and contents remain distinct.

12. A project tab summarizes activity with three independent numeric badges: a green count for Claude Code and Codex turns currently working, a blue count for unread completed turns from those agents, and a visually muted count for ordinary foreground terminal commands currently running. Merely opening an interactive agent process, leaving it idle, or interacting with its startup UI does not contribute to the green count. Claude Code and Codex contribute to the same agent counts and are excluded from the ordinary-command count. Agent-driven commands and read-only terminal viewers are also excluded from the ordinary-command count. All other foreground commands contribute while actively executing; a command that starts and finishes before the UI can present a running frame may never visibly increment the count. While an ordinary command is running, its inner tab keeps the existing purple in-progress treatment and also shows a muted gray dot in the trailing notification-dot position. The gray dot is present in expanded, compact, and summary tab layouts. The blue count uses the existing fixed notification blue and corresponds to the blue unread dots on the completed turns' inner tabs. When the user explicitly interrupts a working Claude Code or Codex turn with Escape or Ctrl-C, its green per-agent activity dot and hover status change to blue **Done** until the next turn starts. Because the user initiated and observed that interruption, it does not create an unread-completion badge or a completion toast.

13. When a command represented by the muted running count completes, it creates a terminal-scoped notification and the existing generic blue unread dot; failures are represented as errors in the notification mailbox. If the originating tab is visible in the active window, the blue dot remains visible for approximately one second and then clears automatically if the user is still viewing that tab, without showing an in-app toast. If the user switches away during that interval, the notification remains unread. A completion that originates outside the visible tab remains unread and participates in the same in-app toast, mailbox, focus-to-read, and project-aware navigation behavior as agent notifications. It does not increment the blue completed-agent count. Starting the next command in that pane clears its stale command-completion notification, and closing the pane removes it. Existing configured desktop-notification thresholds continue to govern native OS notifications.

14. Activating a project alone does not clear its blue completed count or notification dot. Each indicator disappears only when its underlying notifications are read, removed, or otherwise cease to be unread according to the existing notification behavior. Clicking a desktop or in-app notification for a session or command in an inactive project first activates the containing window and project, then activates and focuses the originating inner tab and pane. It must not navigate to a similarly positioned pane in the currently active project.

15. Pressing `Command+N` while a normal Clinch window is active creates and activates a new project at the end of that window's project strip. The new project starts with the same empty/default workspace that a new Clinch window starts with today.

16. Pressing `Command+N` when no normal Clinch window is available creates a normal window containing one new project. Quake-mode, onboarding, transient drag-preview, and other special-purpose windows are not used as project containers.

17. The application menu labels the `Command+N` action as **New Project**. An explicit **New Window** command remains available for users and system integrations that require a separate OS window; it creates one window containing one project.

18. `Command+]` activates the next project and `Command+[` activates the previous project. Navigation wraps from the last project to the first and from the first to the last. With only one project, both commands are no-ops.

19. Project switching does not change the existing inner-tab shortcuts (`Shift+Command+}` and `Shift+Command+{`). On macOS, the project shortcuts replace the previous `Command+[` / `Command+]` pane-navigation and rich-text indentation defaults so the project action always owns those chords. Pane navigation remains customizable through keybinding settings. Project shortcuts are discoverable in menus/keybinding settings and may be customized through the existing keybinding system.

20. Project tabs can be reordered within a window by dragging horizontally. Reordering preserves the active project, all project contents, unread state, and most-recently-used state.

21. Dragging a project tab away from the project strip creates a floating preview that follows the pointer. Dropping it on empty desktop space detaches that same live project into a new normal Clinch window at the drop location; sessions are transferred, not restarted or restored from a snapshot.

22. Dragging a project tab over another normal Clinch window's project strip shows an insertion indicator. Dropping attaches the same live project at that position in the target window and focuses it. A project cannot be dropped into a quake-mode, onboarding, transient preview, or otherwise incompatible window.

23. If the dragged project is the source window's only project, dragging on empty desktop space moves that window rather than creating an empty source window. Dropping it into another compatible window closes the now-empty source window after the live project has attached successfully.

24. Cancelling a project drag with Escape or returning it to its source strip leaves the project in the source window at a valid position. A failed transfer also returns the project to the source without losing sessions, tabs, panes, or unread state.

25. A project tab exposes a close button using the same hover and active-state conventions as existing tabs. Closing a project closes all of its inner tabs as one operation and uses the existing running-session/close-confirmation protections before destructive teardown.

26. Closing the active project activates the nearest surviving project: prefer the project that shifts into the closed position, otherwise the previous project. Closing the only project closes the OS window through the existing window-close flow.

27. `Command+W` continues to close the active inner tab/session, and `Shift+Command+W` continues to close the window. Neither shortcut is silently repurposed to close a project in the first version.

28. When project tabs exceed the available header width, tabs shrink to a usable minimum and then become horizontally scrollable or use Clinch's standard overflow affordance. The active project is always scrolled into view, and toolbar/window controls never become inaccessible.

29. Project switching is keyboard navigable through the project shortcuts. Close and left/right reorder actions are exposed through command-palette/keybinding actions, and accessibility output describes every project's label, selected state, position, and unread state while announcing project actions. Focus remains in the active project's content after mouse or shortcut switching.

30. Window-level visual state follows the active project where it is currently derived from the active workspace, including the repository-based header tint and OS window title. Switching projects must not allow a background project to overwrite those active-window values.

31. Session restoration persists physical windows, the ordered projects in each window, the active project, and every project's existing workspace snapshot. After restart, each project returns to the same containing window and order, with the same active inner tab and persisted workspace state.

32. Saving or restoring during a project drag never persists a source/preview duplicate. Persistence records either the last committed source arrangement or the completed target arrangement.

33. Existing actions that explicitly target the active window continue to operate on the active project unless they explicitly request a new project or new window. Actions targeting a pane or terminal by identity locate and activate its containing project before performing the action.

34. Special windows that do not represent a normal terminal workspace—quake mode, authentication/onboarding, web-only simplified views, and transient transfer previews—retain their current behavior unless explicitly made project-compatible later.

35. Clinch Settings includes a **Create new tabs in Git worktrees** switch under a Projects category. The switch is on by default and is stored as a local Clinch preference.

36. When that switch is on and the user creates an ordinary terminal tab inside an existing project, Clinch resolves the active tab's local Git repository and creates a new branch and linked worktree for that repository. The new branch starts from the local `main` branch when it exists, otherwise from `origin/main` when that remote-tracking ref exists.

37. Worktree creation applies to the primary new-tab action and the same action when used inside a tab group, including an ordinary built-in agent tab selected as the default new-tab mode. For an agent tab, worktree setup finishes before Agent Mode starts. It does not reinterpret explicit new-terminal or alternate-shell actions, restored tabs, split panes, cloud-agent tabs, Docker sandbox tabs, user-authored tab configurations, notification/deep-link destinations, local-control requests, or other feature-specific tab creation flows.

38. Clinch generates a unique, readable branch name without prompting, creates the linked checkout under Clinch's existing managed worktree directory, and opens the new shell from the source repository while the existing worktree setup commands run. Concurrent new-tab actions must select different available branch names and paths.

39. After a terminal enters a linked Git worktree, its inner-tab row shows a non-exclusive **Worktree** chip with Clinch's existing worktree icon. The chip renders in both expanded and compact vertical-tab layouts without replacing agent, error, sharing, unsaved-state, unread, diff, or pull-request indicators. The tab's existing branch metadata continues to show the worktree branch name, and `Worktree` is included in tab search and accessibility output. This indicator also appears for linked worktrees opened through existing manual flows, not only worktrees created by this setting.

40. If the setting is off, the current new-tab behavior is unchanged. If the active tab is not a local terminal in a Git repository, the repository has neither `main` nor `origin/main`, Git is unavailable, or worktree preflight otherwise cannot resolve a safe source checkout, Clinch opens the requested ordinary tab instead; it never guesses another base branch or mutates the current checkout.

41. If Git reports an error while the queued worktree setup command is running, the error remains visible in the new terminal and the shell stays usable from the source repository. The Worktree chip does not appear unless the terminal actually enters a linked worktree. No existing branch, worktree, project, or tab is removed automatically, and existing manual worktree/tab-config flows retain their current behavior.

42. The vertical-tabs control row includes a compact worktree-icon toggle immediately before the new-tab button. It mirrors the same global **Create new tabs in Git worktrees** preference as Clinch Settings rather than introducing project-specific state. Its active and inactive appearance reflects the current value, and its tooltip states whether automatic worktree tabs are on or off and that the choice applies to all projects. Changing the preference from either surface updates the other.

43. An ordinary inner tab may be turned into its own project when its source project has at least one other inner tab. Dragging the tab upward into the project header shows a project-position insertion indicator; dropping creates and activates a project at that position around the same live tab. The terminal, editor, pane layout, agent state, custom title, and panel state are transferred rather than restarted or reconstructed.

44. An eligible inner tab's right-click menu includes **Move Tab to New Project**. Choosing it creates and activates the new project immediately after the source project. The item is hidden when moving the tab would leave its source project empty or when the containing window does not support project tabs.

45. The whole compatible project-header surface is a promotion target so the user does not have to aim at a narrow project pill. Dragging an inner tab elsewhere retains the existing reorder and cross-window behavior, including detaching it into a new OS window; merely crossing the inner-tab strip boundary does not promote it unless it is dropped in the project header.

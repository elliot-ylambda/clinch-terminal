# Sidebar Sections and Tasks

## Summary

Clinch's vertical sessions panel lets people organize open sessions into named visual sections and
keep a lightweight project task list. A task can be converted into a new Claude Code or Codex
session with one action, while sections and tasks survive application restarts and mirror into a
connected Clinch Remote Control client.

## Goals and non-goals

- Sections are visual organization only. They do not schedule work, target prompts, or change how
  sessions execute.
- Tasks are a small inbox, not a project-management system. There are no due dates, assignees,
  priorities, run histories, or automatic completion tracking.
- Persistence is local-first. Remote Control mirrors the running Mac; offline and cross-Mac cloud
  synchronization are out of scope.

## Figma

Figma: none provided. The existing vertical-tab group cards and Remote Control drawer establish the
visual language for this MVP.

## Behavior

### Session sections

1. The vertical sessions panel supports named sections within each project. A section belongs only
   to the project in which it was created.

2. A full-width **Create new section** button is pinned to the bottom of the vertical sessions
   panel, below the Tasks area. Activating it creates a normal new session as the first member of a
   new section and immediately enters section naming. People can also create a section from the
   new-session menu, from an existing session, or from a multi-selection of sessions. A section is
   removed automatically when its last session leaves.

3. A newly created section immediately enters inline naming. Section names must contain at least
   one non-whitespace character; canceling or submitting an empty name keeps the prior/default
   name.

4. Section cards display a clear outline, the section name, the number of contained sessions, and
   an expand/collapse affordance. Their overflow menu offers six distinct color choices plus an
   uncolored default; a selected color appears as a light card tint and colored outline rather than
   a saturated fill. Hovering a section reveals a plus action that creates a normal new session
   directly in that section; if the section is collapsed, it expands to reveal the new session.
   Collapsing a section hides its session rows without closing or suspending them.

5. People can drag sessions to reorder them within a section, drag them into another section, or
   drag them into the unsectioned area. A collapsed section header accepts a dropped session.

6. People can drag a section card to reorder the complete section as one contiguous block.

7. Removing a section keeps every contained session open and moves them to the unsectioned area.
   Closing every session in a section remains a separate, explicitly labeled destructive action.

8. Existing session behavior is unchanged for people who do not create sections. Search, active
   session highlighting, pinned items, closing sessions, and project switching continue to work.

9. Section names, membership, order, collapsed state, and selected color survive quitting and
   restarting Clinch.

### Tasks

10. An expanded **Tasks** area appears below the session list for the current project. It has a
   clear green top divider matching the thickness, brightness, and opacity of the **Create new
   section** button outline, an open-task count, a collapse affordance, and an **Add a task...**
   single-line input.

11. Entering non-whitespace text and pressing Enter adds one task to the end of the current
    project's list, clears the input, and saves the workspace. Empty input does nothing.

12. Each task displays its text and three actions: start in Claude Code, start in Codex, and remove.
    The provider actions have accessible labels and recognizable provider icons.

13. Starting a task creates a normal session in the same project, uses Clinch's normal new-session
    working-directory behavior, and submits the complete task text as the CLI agent's initial
    prompt. The new session follows the same section-inheritance behavior as any other new session.

14. A task is removed only after Clinch has created the new agent session and attached its launch
    command. If launch cannot be initiated, the task remains available and Clinch reports the
    failure rather than silently losing it.

15. Removing a task never closes a session and does not affect session sections. Converting a task
    does not create ongoing synchronization between the task and the resulting session.

16. Task order, text, and the Tasks area's collapsed state survive quitting and restarting Clinch.
    Tasks remain isolated by project.

17. Task text stays local to Clinch and authenticated Remote Control connections. Clinch does not
    add task contents to telemetry.

### Remote Control

18. The connected Remote Control drawer groups session rows under the same section names shown on
    the Mac. Ungrouped sessions remain visible under the normal open-sessions area.

19. The selected project's open tasks appear in a Tasks area in the Remote Control drawer in the
    same order as the Mac. Changes made on either surface appear after the next authoritative
    workspace snapshot.

20. Remote Control can add and remove tasks and start a task in Claude Code or Codex. These actions
    operate on the real project on the Mac; the phone never creates mobile-only task or session
    state.

21. Remote task mutations require a live authenticated connection and a current workspace
    revision. Starting a task additionally requires session-creation permission. Stale, missing,
    or already-converted task IDs fail safely and refresh from the Mac's authoritative state.

22. When the Mac is asleep, offline, or Clinch is not running, Remote Control may show its last
    received snapshot as unavailable but cannot edit, queue, or launch tasks for later.

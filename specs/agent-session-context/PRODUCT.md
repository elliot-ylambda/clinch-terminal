# Claude and Codex Session Context

## Summary

Give every recognized Claude Code and Codex session a useful, stable identity derived from the
first message the user sent. Show that identity in the tab title, show the first and most recent
user messages in a compact header above the session, and provide a `Message history` popover that
survives quitting and resuming Clinch.

## Problem

Claude Code and Codex tabs commonly fall back to a repository or directory name, so several agent
sessions in the same project are hard to distinguish. After selecting a tab, the terminal also
provides no persistent summary of how the session began or what the user most recently asked.

## Goals

- Make Claude Code and Codex sessions recognizable without manually renaming every tab.
- Keep the tab's default identity stable while the conversation continues.
- Make the beginning, current request, and complete user-message history quickly inspectable.
- Restore the same context when Clinch restores or resumes the underlying agent session.

## Non-goals

- Displaying or duplicating assistant responses; this feature records user-authored messages only.
- Replacing the Claude Code or Codex transcript viewer.
- Changing the default title behavior of plain terminals, Oz conversations, or other CLI agents.
- Changing Warp-branded builds; this is Clinch session chrome and storage behavior.
- Syncing message history to a Clinch service or another device.
- Adding message editing, deletion, search, or branching in the first version.

## Figma

Figma: none provided.

## Behavior

### Session identity and tab titles

1. A recognized Claude Code or Codex session receives a derived session title as soon as Clinch
   knows its first non-empty user-authored message.

2. The derived title is made from that first message as follows:
   - Leading and trailing whitespace is removed, and runs of whitespace or line breaks are shown as
     a single space.
   - If the first sentence ends within the first 80 visible characters, the title is that sentence.
   - Otherwise, the title is the first 80 visible characters followed by an ellipsis.
   - Generated environment, permission, skill, or other machine-authored context that an agent may
     place in its transcript is not treated as the user's first message.

3. The complete first message remains stored and available in the session header and history even
   when the derived title is shortened.

4. Before a first message is available, the tab keeps its existing repository, directory, terminal,
   or agent fallback title. The title updates in place when the first message becomes available; the
   user does not need to change tabs or restart the session.

5. By default, follow-up messages do not change the derived title. This gives the session a stable
   label while the most recent request changes independently in the header.

6. A user-assigned tab title remains the highest-priority title. Receiving a message, restoring
   history, or resuming the session never overwrites a manual title. Clearing the manual title
   reveals the current derived or fallback title.

7. An existing explicit preference to show the latest prompt in agent tab names remains respected
   on the tab surfaces to which that preference already applies. With that preference disabled (the
   default), Claude Code and Codex use the stable first-message title.

8. Tab title clipping keeps the beginning of a derived message visible and exposes the complete
   first message in the tab tooltip. Directory-style titles retain their existing start-clipping
   behavior.

9. In a split tab, each Claude Code or Codex pane has its own session identity. The tab title follows
   the focused pane, using the same precedence rules as existing split-pane titles.

### In-session context header

10. When the focused pane contains an active or resumed Claude Code or Codex session with at least
    one known user message, Clinch shows a compact context header directly above the terminal
    content. Plain terminals and unsupported agents do not show this header.

11. With two or more messages, the header shows:
    - `Started with` followed by a width-constrained preview of the first message.
    - `Latest` followed by a width-constrained preview of the most recent message.
    - A `Message history (N)` button, where `N` is the number of known user messages.

12. With exactly one message, the header shows it once as `Started with · Latest` and shows
    `Message history (1)`. It does not render duplicate first/latest text.

13. Header previews preserve the beginning of each message, collapse display-only whitespace, and
    end with an ellipsis when they do not fit. Hovering a preview exposes the complete message.

14. The `Latest` preview updates as soon as Clinch observes a newly submitted user message. The
    stable first-message title and `Started with` preview do not change.

15. Selecting a tab or focusing a split pane updates the visible context header immediately. The
    header does not take keyboard focus from the terminal merely because the tab or pane became
    active.

16. A loading state may appear while a resumed session's history is being recovered. It is compact,
    does not block terminal interaction, and is replaced in place when recovery finishes.

### Message history

17. Activating `Message history (N)` opens an anchored, scrollable popover that lists every known
    user-authored message for that agent session in chronological order, oldest first.

18. Each history entry shows its turn number, full message text with original line breaks, and its
    submission time when that time is available. Repeated messages remain separate turns.

19. Message text is shown in full, including original line breaks. Long histories scroll within a
    bounded popover instead of resizing the terminal or window. A dedicated selectable-text,
    export, or search surface is outside the first version.

20. The history button is keyboard accessible. Enter or Space opens it, arrow-key navigation and
    normal scrolling can inspect its contents, and Escape closes it and returns focus to the
    terminal pane. Clicking outside also closes it.

21. Opening or closing history does not send input to the agent, alter terminal selection, or change
    the session's title.

### Capture, restoration, and failure handling

22. History belongs to the provider session, not to an ephemeral tab or view identifier. Moving a
    tab, reordering tabs, changing the focused split, or recreating the view for the same resumed
    session does not create a new history.

23. User messages observed while the session is live are added once and in submission order. A
    completion/stop notification that repeats the current query does not create a second history
    entry.

24. After Clinch quits and relaunches, restoring or resuming the same Claude Code or Codex session
    restores its first message, latest message, message count, and complete known user-message
    history. The restored tab title follows the same rules as a continuously running session.

25. If a message is submitted while older history is still loading, the loaded and live messages
    are merged without losing the new message or duplicating an existing turn. Results from a stale
    load for a previous session are ignored.

26. Starting a genuinely new agent session in a reused pane clears the previous session's visible
    context and begins a new title/history after the new first message. Resuming an existing session
    keeps that session's prior context.

27. Only the outer agent session that owns the Clinch pane controls its tab title and context
    header. A nested Claude Code or Codex process cannot replace the outer session's visible
    identity or mix its messages into the outer history.

28. Existing sessions created before this feature recover as much context as is locally available.
    A session with only its first message recoverable shows a one-message header; missing timestamps
    are simply omitted.

29. Missing, malformed, truncated, or temporarily unreadable history never prevents the pane from
    opening or the agent from resuming. Clinch keeps the normal fallback title when no messages can
    be recovered, shows an unobtrusive partial state in the history control when some recovered
    history may be incomplete, and continues capturing new messages.

30. Empty or whitespace-only submissions are ignored. Arbitrarily long or multi-line messages are
    retained up to the same bounded local-history safety limit used by Clinch's existing agent
    recovery data; reaching the limit is represented as partial history rather than silently
    pretending the history is complete.

31. The feature's durable data remains local to the machine, uses the same private-file treatment
    as Clinch's existing agent-resume records, and is included in the same local recovery/update
    snapshots. Enabling this feature does not upload prompt text.

32. If structured live metadata is unavailable, Clinch uses locally recoverable agent history when
    it can identify the session. If neither a session identity nor recoverable history is available,
    the pane continues to behave like today's directory/repository-titled session rather than
    guessing or showing stale text.

33. Warp-branded builds retain their existing CLI-agent tab and pane behavior. The new context
    header and Clinch prompt-history recovery path are enabled only in Clinch builds.

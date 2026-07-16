# Two-way iMessage agent notifications

> Shipping status: disabled. Current Clinch builds do not compile or expose this feature and do
> not include its helper or macOS permission metadata. This document is retained for a possible
> future opt-in build.

## Summary

Clinch can use the signed-in Messages account on the user's Mac to notify their iPhone when a durable Codex or Claude Code CLI turn finishes and to route text replies back into the correct live CLI session. The feature is local, opt-in, and requires no iPhone app or hosted service.

## Goals and non-goals

- The first release supports durable Codex and Claude Code CLI sessions on macOS 14 or newer.
- It does not support Clinch's native agent, generic terminal processes, ID-less fallback detections, attachments, reactions, SMS fallback, or remote interaction with permission and confirmation prompts.
- Messages content and the destination phone number remain local to the Mac and are never sent to Clinch telemetry or a Clinch backend.

## Figma

Figma: none provided. The controls follow existing Clinch Settings, pane-header, and CLI-agent-footer patterns.

## Behavior

1. Clinch exposes a global iMessage connection status in the header, a prominent green-outlined iMessage control in every supported durable Codex and Claude Code footer, and setup and troubleshooting controls in Clinch Settings. Before setup, the footer action is labeled **Set up iMessage** and opens the iMessage Settings category. After setup, the same control becomes a two-state **Get notified: Yes/No** toggle for that exact session.

2. The feature is disabled until setup succeeds. Settings begins with **Set up your phone**, a phone-number field, and an explicit **Connect** action (Return is the keyboard equivalent). Submitting a valid number persists it immediately, replaces the input with a pill containing that number, updates supported agent footers from **Set up iMessage** to the current setup state, and explains that messages use the Messages account already signed in on the Mac. It does not ask the user to install or configure an iPhone app.

3. After a number is submitted, Settings shows a persistent setup checklist rather than only one opaque status. The checklist contains **Phone number**, **Full Disk Access**, **Allow Clinch to control Messages**, and **Signed in to iMessage**. Each requirement has a non-color status: checking, complete with a check mark, or incomplete. Clicking an incomplete requirement opens the relevant System Settings pane or Messages, or requests the relevant macOS permission. Returning to Clinch never discards the number, and **Check again** rechecks all requirements when a permission change has not been detected automatically.

4. **Test iMessage** is visible as soon as a number has been submitted. It is disabled, with an explanation, while any send prerequisite is incomplete or still being checked. When every checklist requirement is complete, selecting Test sends a unique setup message and asks the user to reply from the iPhone. The button and nearby copy distinguish **Sending**, **Sent—reply with the code**, and **Failed—try again**. Clinch never says **Check your phone**, **Sent**, or otherwise implies delivery until Messages has acknowledged the send and Clinch has correlated the sent message in the local database. A failed or delivery-indeterminate attempt remains retryable through the same button.

   Clinch enables the feature only after it observes the reply in the same Messages conversation and verifies that it can both send and receive there. If it observes a supported text reply that does not match the one-time code, Settings, the header, and agent footers explicitly say that the reply was received but did not match and ask the user to resend the code exactly; setup remains pending and a later correct reply succeeds. Once setup is complete, Test sends a short standalone message and does not alter the connection or agent notification preferences.

   While this is in progress, Settings, the header, and supported agent footers distinguish checking requirements, ready to test, sending, waiting for the phone reply, missing permission, and failure. Starting setup over is exposed as **Change number**; it clears conversation-specific GUID history, retained route selections, and queued phone replies before calibrating the new conversation. It preserves live session identities and explicit per-session notification overrides, and Settings discloses the clearing behavior.

5. After successful setup, Settings says **Connected to this number** beside the persisted number pill, keeps the completed checklist available for troubleshooting, and exposes **Test** and **Change number** actions. Messaging and **Get notified by default** are on initially for all currently live and subsequently created durable Codex and Claude Code sessions. Changing the default immediately updates sessions that still inherit it. Toggling a footer writes an explicit Yes or No override for that durable session, and that override is preserved across restarts and later default changes.

6. Disabling the global feature stops new notifications and inbound routing without deleting setup information, the notification default, or per-session overrides. Footer toggles continue to show and edit each session's saved preference while the global feature is paused, and their tooltip makes the global pause clear. Disconnecting or changing the number stops the listener and removes the stored destination, calibrated conversation, pending selections, and queued replies while preserving the user's notification default and per-session overrides for a later setup.

7. When an enabled session changes to a successfully finished state, Clinch sends the complete trustworthy structured final response. It never scrapes arbitrary terminal output to manufacture a response; if structured output is unavailable it sends a generic completion message.

8. Each completion identifies the provider, project or working directory when available, and a short stable route code such as `C7K2`. Codes always mix letters and digits so ordinary four-letter replies are not mistaken for routing instructions. The code remains stable for that durable agent session.

9. A response too long for a comfortable iMessage is sent as ordered, numbered plain-text parts. The complete response is delivered without silently truncating content, and a reply to any part addresses the same route.

10. Clinch sends completion messages only for successful finished turns. It does not notify for or remotely answer permission requests, confirmation prompts, questions represented as blocked terminal state, or other live TUI interactions.

11. An incoming text reply is associated with a session, in priority order, when it is an iMessage reply to a known completion part, begins with a live route code, or exactly one enabled live session is eligible. A recognized route prefix is not included in the text submitted to the agent.

12. When more than one session could receive a code-free message, Clinch retains the original message temporarily, texts back a concise list of route codes and session labels, and lets the user answer with only a route code. The selected route receives the retained original text.

13. Ambiguous retained messages expire after ten minutes. A local expiry timer produces a body-free explanatory iMessage; an unknown route code or no eligible session likewise produces an explanation and never guesses a destination.

14. A reply to a finished, live session starts an ordinary follow-up turn using the same input behavior as submitting text from Clinch's CLI-agent footer.

15. If the destination agent is already working, has an earlier queued reply, has a completion notification awaiting delivery, or has accepted a phone submission whose working-state update has not arrived yet, Clinch queues the reply rather than typing into the live TUI. Queued replies are submitted in FIFO order, one after each successful completion, and a local expiry timer cancels them after 24 hours.

16. Clinch never injects a phone reply into a blocked prompt. A queue waits for a successful completion or is cancelled if the session can no longer safely receive it.

17. Before every submission, Clinch verifies the exact provider and durable session ID, the current owning pane, and a live agent process. Pane IDs alone are never sufficient for routing.

18. If a session exits, changes identity, is opted out, or cannot be resolved, its pending replies are cancelled and the user receives an explanatory iMessage. They are never redirected to another pane or a newer session.

19. Queues, route associations, incoming-message deduplication, and database progress survive an ordinary Clinch restart. A restart does not inject the same phone message twice.

    If secure setup remains but the local database cursor is missing, corrupt, or from an unsupported state version, Clinch fails closed and requires a fresh calibration rather than reading the calibrated conversation from its historical beginning.

20. Clinch watches only the calibrated conversation. It ignores messages it sent itself, duplicate events, reactions, edits, empty bodies, and unsupported attachments.

21. Because replies in a self-conversation may be synchronized as messages sent by the user's own account, Clinch identifies its own output by the exact Messages GUIDs it recorded instead of discarding every `is_from_me` row. Before each send it persists a short-lived SHA-256 text fingerprint and cursor, allowing a restart between send acceptance and GUID persistence to recover that one outgoing row's exact GUID without storing another response-body copy.

22. Setup clearly discloses that Messages does not expose a supported, trustworthy source-device identity. Any Apple device synchronized with the configured Messages account and conversation may be capable of issuing a reply; Clinch cannot cryptographically prove it came from the iPhone.

23. Revoking Automation or Full Disk Access pauses the bridge, preserves undelivered local state, changes the corresponding checklist requirement to incomplete, and shows the same remediation in the header, footer, and Settings. Restoring permission resumes without duplicate delivery.

24. Phone numbers, message bodies, queued replies, and database-derived identifiers are absent from analytics, crash metadata, and ordinary logs. Diagnostics may contain redacted counts, status categories, and opaque locally generated route codes.

25. All controls have accessible labels, keyboard focus behavior matching adjacent controls, and non-color-only connected, paused, error, and disabled states.

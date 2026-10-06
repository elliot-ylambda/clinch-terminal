# Background messaging and recent-history proof

Date: 2026-09-30. Implementation branch: `codex/clinch-recent-messages`.

The installed production app responded to `app ping` but did not recognize `agent`.
This test therefore used the previously signed project-control development build,
with a fresh data profile and isolated control discovery. The production app stayed
running. No existing user session received a prompt or was moved or closed.

One real interactive Claude Code session was launched with Clinch's native provider
notification hooks, the user's existing authentication, and tools disabled. Its
conversation ID was `8e1808e9-f727-479f-b7f7-2aa8f763172e`. This was a provider
conversation, not a simulated transcript. It first replied `READY_5587ebc13a44`.

Before sending the test prompts, the foreground project was set to
`aa426cf0-c1a7-4796-9aba-591f9c15d2cb`, tab `1747` (“Foreground — stay here”).
The Claude worker was in the other project, `48321d00-3c42-4e9d-ae99-2bcf666f1445`,
tab `4217` (“Background messaging proof — Claude”).

For each send, the CLI inspected the exact agent, required readiness, and supplied
its current input revision and a fresh request UUID. The text arrived as a user
prompt in that same Claude conversation. The CLI reported a submitted receipt;
subsequent transcript reads independently confirmed the actual provider reply.

| Request ID | Actual reply | Reply timestamp (UTC) |
| --- | --- | --- |
| `669af28d-ae6b-4c6f-9f03-75885edea92a` | `BACKGROUND_REPLY_A_5587ebc13a44` | 2026-09-30T22:14:30.791Z |
| `86dc3636-c107-414e-8d73-e5fa1e9fa501` | `BACKGROUND_REPLY_B_5587ebc13a44` | 2026-09-30T22:15:14.977Z |

`workspace tree` snapshots before and after the sends and reads showed the same
foreground project and tab. No activate command was used during delivery or reading.

Existing `agent read --tail --limit N` produced exactly:

| N | Records, oldest to newest |
| --- | --- |
| 1 | Assistant reply B |
| 2 | User prompt B; assistant reply B |
| 3 | Assistant reply A; user prompt B; assistant reply B |

Receipts, native read responses, and selection snapshots are saved locally under
`/tmp/cbg-d4bhazq4`; the test setup is `/tmp/clinch-background-proof.py`. These paths
are temporary local evidence, not repository fixtures. Shared development settings
were checked by hash and remained unchanged.

The newly built CLI was then tested against this same running app and conversation:

- No read options returned the latest three records.
- `--last 1` and `--last 2` returned one and two recent records.
- `CLINCH_AGENT_READ_LIMIT=2` changed the default to two; `--last 1` overrode it.
- The existing `--tail --limit 2` syntax still worked.
- `--from-start --limit 2` read the initial exchange; its cursor read the next exchange.
- `--output-format ndjson agent read ... --all --limit 2` returned all six supported
  records exactly once: three nonempty pages and one final empty page that scanned
  trailing provider metadata and established EOF. Every page retained coverage metadata.
- An invalid configured limit of zero failed with exit code 2.
- The selected project and tab remained unchanged throughout these reads.

Results are in `/tmp/cbg-d4bhazq4/new-cli-results.json`; the repeatable read check is
`/tmp/clinch-recent-read-proof.py`. The real provider session was left open in the
isolated Clinch Dev app for inspection. This does not update the installed production app.

Automated validation: all 196 `warp_cli` library tests passed, including recent/history
mode selection, count bounds, streaming pagination, partial provider records, stalled
cursors, and the streaming-output requirement. `warp_cli` Clippy with warnings denied,
repository formatting, whitespace checks, and the app executable build passed. The
full workspace/release gate was not run. This was pre-merge development validation, not a packaged release. Later extension
validation is recorded in [CLI_COMPLETION_TEST.md](./CLI_COMPLETION_TEST.md).

Counts refer to normalized transcript records, including tools. Coverage is explicitly
`partial`: images, reasoning, and unsupported provider metadata are omitted. This
run verifies Claude; real Codex coordination evidence is recorded in [LIVE_TEST.md](LIVE_TEST.md).

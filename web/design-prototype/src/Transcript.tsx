import type { ElementType, ReactNode } from "react";

import type { Tab, Transcript } from "./data";

// Terminal content. Everything inside the terminal card is drawn by the program running there
// (Claude Code, Codex, or the shell); Clinch only controls its font and colors. The prototype
// imitates that output so the chrome around it is judged in context.

const Bullet = ({ tone = "text", children }: { tone?: "text" | "tool" | "claude"; children: ReactNode }) => (
  <div className="line">
    <span className={tone === "tool" ? "ok" : tone === "claude" ? "claude" : "strong"} aria-hidden="true">
      {tone === "claude" ? "✻" : "●"}
    </span>
    <span>{children}</span>
  </div>
);
const C = ({ children }: { children: ReactNode }) => <span className="code">{children}</span>;

export function TranscriptView({ tab, find }: { tab: Tab; find: string }) {
  const body = TRANSCRIPTS[tab.transcript](tab);
  return <div className="stack">{find ? highlight(body, find) : body}</div>;
}

function highlight(node: ReactNode, needle: string): ReactNode {
  // Light-touch find highlighting over plain string children.
  if (typeof node === "string") {
    const parts = node.split(new RegExp(`(${needle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")})`, "gi"));
    return parts.map((p, i) => (p.toLowerCase() === needle.toLowerCase() ? <mark key={i}>{p}</mark> : p));
  }
  if (Array.isArray(node)) return node.map((n, i) => <span key={i}>{highlight(n, needle)}</span>);
  if (node && typeof node === "object" && "props" in node) {
    const { type, props } = node as unknown as { type: ElementType; props: Record<string, unknown> };
    if (props.children === undefined) return node;
    const Tag = type;
    return <Tag {...props}>{highlight(props.children as ReactNode, needle)}</Tag>;
  }
  return node;
}

const TRANSCRIPTS: Record<Transcript, (tab: Tab) => ReactNode> = {
  shopify: () => (
    <>
      <Bullet>
        One emulated contract test fails: injecting a Shopify failure during product creation yields <C>draft</C> instead of the
        expected <C>upstream_failed</C>, likely because the test still targets the old REST endpoint after the PR’s switch to
        GraphQL.
      </Bullet>
      <div className="dim indent">Ran 1 shell command</div>
      <Bullet>
        Confirmed: this test came from main after Codex’s branch point and injects failure on REST POST <C>products.json</C>,
        which this PR no longer calls.
      </Bullet>
      <Bullet tone="tool">
        <span className="strong">Bash</span>(cd …/magister-marketing/claret-rattler &amp;&amp; python3 - &lt;&lt;'PY' …)
        <br />
        <span className="dim">
          ⎿ 32: emulator_host.seed('shopify', 'faults', method='POST', path='graphql\\.json$', status=404)
          <br />
          {"  "}exit=2 · tests/emulated/test_mcp_content_provider_writes.py:21: AssertionError
        </span>
      </Bullet>
      <Bullet>
        CodeRabbit raised no new issues on <C>5cfd79635</C>. The test now passes the injected failure, but the approved publish
        finishes <C>running</C> with an “outcome unknown” message instead of <C>succeeded</C>.
      </Bullet>
      <Bullet>
        I see the issue — the test’s seeded token only has <C>write_products</C>, but the publish path needs{" "}
        <C>read_publications</C>/<C>write_publications</C> like the real OAuth grant. I’ll update the test seed to match.
      </Bullet>
      <div className="dim indent">Running 1 shell command…</div>
      <Bullet tone="claude">
        <span className="claude">
          Zesting… <span className="dim">(1m 58s · ↓ 5.5k tokens · thought for 6s)</span>
        </span>
      </Bullet>
    </>
  ),

  answer: () => (
    <>
      <Bullet>
        I’d keep Clinch’s codebase and give it a real visual redesign. Of the three options you named, Electron is the only one
        that would make Clinch slower, and forking Ghostty would mean rewriting every feature we’ve built.
      </Bullet>
      <div className="indent strong">The options</div>
      <div className="indent">
        <span className="strong">1. Rebuild in Electron: I wouldn’t.</span>
        <br />
        <span className="dim">–</span> It would be slower, not faster. Clinch draws everything on the GPU from Rust.
      </div>
      <div className="indent">
        <span className="strong">2. Fork Ghostty: a beautiful base, but a rewrite.</span>
        <br />
        <span className="dim">–</span> Every Clinch feature would be rebuilt in Swift and Zig.
      </div>
      <div className="indent">
        <span className="strong">3. Redesign Clinch where it is: recommended.</span>
        <br />
        <span className="dim">–</span> Corner sizes are hard-coded in about 400 places. One set of design tokens fixes that.
      </div>
      <Bullet>Pick a direction and I’ll build mockups of each state.</Bullet>
    </>
  ),

  coordinator: () => (
    <>
      <Bullet>
        Watching <span className="strong">23 sessions</span> in magister-marketing. Since your last message:
      </Bullet>
      <div className="indent">
        <span className="ok">✓</span> <span className="strong">Facebook integration OAuth connection</span> finished; PR #2163
        is green.
        <br />
        <span className="ok">✓</span> <span className="strong">Harness bench S1</span> finished; nothing to merge.
        <br />
        <span className="err">!</span> <span className="strong">Magister gateway HTTP 5xx spikes</span> is waiting on a
        permission prompt.
        <br />
        <span className="err">!</span> <span className="strong">Sentry errors from Magister alerts</span> and{" "}
        <span className="strong">AI visibility</span> both want to deploy the gateway.
      </div>
      <Bullet>
        Two sessions want to deploy the same service, so I’ve queued one deploy after both PRs merge instead of two. Approve it
        and I’ll tell both sessions to hold.
      </Bullet>
      <div className="tui-box">
        <div className="strong">Proposed: one gateway deploy after #2163 and #2170 merge</div>
        <div className="tui-option sel">❯ 1. Approve</div>
        <div className="tui-option">2. Not now</div>
      </div>
    </>
  ),

  permission: () => (
    <>
      <Bullet>
        The 5xx spikes line up with the gateway’s connection pool hitting its 20-connection cap during the 14:00 cron burst. I
        want to raise <C>POOL_MAX</C> to 40 and redeploy the staging gateway to confirm.
      </Bullet>
      <Bullet tone="tool">
        <span className="strong">Bash</span>(fly deploy --app magister-gateway-staging --strategy rolling)
      </Bullet>
      <div className="tui-box">
        <div className="strong">Do you want to proceed?</div>
        <div className="tui-option sel">❯ 1. Yes</div>
        <div className="tui-option">2. Yes, and don’t ask again for fly deploy commands</div>
        <div className="tui-option">3. No, and tell Claude what to do differently (esc)</div>
      </div>
    </>
  ),

  limited: () => (
    <>
      <Bullet>
        Ultrareview found 3 issues in PR 1905. I fixed the first two and was starting on the openclaw fork comparison.
      </Bullet>
      <div className="err indent">5-hour limit reached ∙ resets 4:20pm</div>
      <div className="dim indent">/upgrade to increase your usage limit.</div>
    </>
  ),

  shell: (tab) => (
    <>
      <ShellBlock cmd="git status --short" cwd={tab.cwd} time="0.04s">
        <span className="ok"> M</span> app/src/workspace/view.rs
        <br />
        <span className="ok"> M</span> app/src/tab.rs
        <br />
        <span className="err">??</span> specs/clinch-redesign/
      </ShellBlock>
      <ShellBlock cmd="git log --oneline -3" cwd={tab.cwd} time="0.02s" selected>
        <span className="code">d5d90c471</span> Fix forked prompt history, fork tab names, and tab status (#101)
        <br />
        <span className="code">ef843335e</span> Update libz-sys for RUSTSEC-2026-0418 (#100)
        <br />
        <span className="code">017eef6a6</span> Pin a coordinator session first in each project’s tab list (#99)
      </ShellBlock>
      <ShellBlock cmd="yarn dev" cwd={tab.cwd} time="running">
        <span className="ok">VITE v8.1.5</span> ready in 412 ms
        <br />
        <span className="dim">➜</span> Local: <span className="code">http://127.0.0.1:5317/</span>
      </ShellBlock>
    </>
  ),

  shellFailed: (tab) => (
    <>
      <ShellBlock cmd="cargo nextest run -p gateway" cwd={tab.cwd} time="48.2s" exit={100}>
        <span className="dim">Starting 412 tests across 6 binaries</span>
        <br />
        <span className="err">FAIL</span> [ 0.212s] gateway tests::emulated::test_mcp_content_provider_writes
        <br />
        <span className="dim">Summary [48.2s] 412 tests run: 411 passed, </span>
        <span className="err">1 failed</span>
      </ShellBlock>
    </>
  ),

  fresh: () => (
    <div className="dim">
      Claude Code v2.1.312
      <br />
      Opus 5.5 · ~/projects/magister-marketing
    </div>
  ),
};

function ShellBlock({
  cmd,
  cwd,
  time,
  exit,
  selected,
  children,
}: {
  cmd: string;
  cwd: string;
  time: string;
  exit?: number;
  selected?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={`block${exit ? " failed" : ""}${selected ? " selected" : ""}`}>
      <div className="block-head">
        <span>{cwd.split("/").pop()}</span>
        <span className="cmd">{cmd}</span>
        {exit ? <span className="exit">Exit {exit}</span> : null}
        <span>{time}</span>
      </div>
      <div>{children}</div>
    </div>
  );
}

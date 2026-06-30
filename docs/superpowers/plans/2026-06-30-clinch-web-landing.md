# Clinch.sh Web App (Landing Page) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up `clinch.sh` as a private-repo, Vercel-deployed Next.js landing page with a Download-for-macOS button to the latest GitHub release.

**Architecture:** A single Next.js (App Router) page in a new private repo `elliot-ylambda/clinch-web`. Section components render from typed content data so copy stays in one place. The download button is a static link to GitHub's `releases/latest/download/Clinch.app.zip`. Deployed on Vercel (scope `elliot-ylambdas-projects`) with git auto-deploy and the `clinch.sh` apex domain.

**Tech Stack:** Next.js 16 (App Router) + TypeScript + Tailwind v4, pnpm, Vercel, `gh` CLI. No unit-test harness — `tsc`/`pnpm build` + `pnpm lint` are the structural gate; live `curl` checks are the functional gate (see Pre-flight and Tasks 6–7).

## Global Constraints

- Project path: `/Users/ellioteckholm/projects/clinch-web` (NOT inside `clinch-terminal`).
- Package manager: **pnpm**.
- Repo: `elliot-ylambda/clinch-web`, **private**.
- Vercel scope: `elliot-ylambdas-projects` (already owns `clinch.sh`).
- Download URL (verbatim, used everywhere): `https://github.com/elliot-ylambda/clinch-terminal/releases/latest/download/Clinch.app.zip`
- Public repo URL: `https://github.com/elliot-ylambda/clinch-terminal`
- Landing page ONLY — no `/docs` route this iteration. Keep the route tree clean for a later `app/docs/` addition.
- All landing copy is repurposed from `clinch-terminal/README.md`; do not invent new product claims.
- Strip all `create-next-app` boilerplate — no sample SVGs, default page text, or starter CSS left behind.
- Visual styling (the terminal-flavored aesthetic) is applied during execution via the `frontend-design:frontend-design` skill; this plan fixes structure, content, and interfaces, not pixel-level design.
- License/attribution must appear in the footer: AGPL-3.0; "Not affiliated with Warp or Denver Technologies, Inc."

---

### Pre-flight: verify the core premise before writing any code

The whole site is built around a static download link to a published release
asset. Confirm that asset actually exists FIRST — if it doesn't, the design's
download button is invalid and the plan must change before Task 1.

- [ ] **Step 1: Confirm the release asset resolves**

```bash
curl -sI "https://github.com/elliot-ylambda/clinch-terminal/releases/latest/download/Clinch.app.zip" | grep -iE "HTTP/|location"
```

Expected: a `302` redirecting toward `…/releases/download/<tag>/Clinch.app.zip`.
If it 404s, STOP — there is no published `Clinch.app.zip` asset; resolve that
(publish a release, or change the asset name in the spec) before proceeding.

- [ ] **Step 2: Confirm tooling**

```bash
node -v && pnpm -v && gh auth status && vercel whoami
```

Expected: Node ≥ 20, pnpm present, `gh` logged in as `elliot-ylambda`, `vercel`
authenticated. (If `vercel whoami` is empty, run `vercel login` — USER authorizes.)

---

### Task 1: Scaffold the Next.js app and strip boilerplate

**Files:**
- Create: `/Users/ellioteckholm/projects/clinch-web/` (whole project via `create-next-app`)
- Modify: `app/page.tsx`, `app/globals.css`, `app/layout.tsx`
- Delete: `public/*.svg` (next.svg, vercel.svg, etc.), any sample assets

**Interfaces:**
- Consumes: nothing (first task).
- Produces: a buildable Next.js app with a blank `/` route and clean `app/layout.tsx` exporting the root layout.

- [ ] **Step 1: Scaffold with create-next-app**

```bash
cd /Users/ellioteckholm/projects
pnpm create next-app@latest clinch-web --ts --tailwind --app --eslint --src-dir=false --import-alias "@/*" --use-pnpm --no-turbopack
cd clinch-web
```

If any flag is rejected by the current generator, run `pnpm create next-app@latest --help` and match: TypeScript, Tailwind, App Router, ESLint, no `src/` dir, import alias `@/*`, pnpm. Confirm Tailwind v4 and Next 16 in `package.json` afterward.

- [ ] **Step 2: Verify the scaffold builds**

Run: `pnpm build`
Expected: build succeeds (compiles `/`).

- [ ] **Step 3: Strip boilerplate**

- Delete every file in `public/` that came from the template (`*.svg`).
- Replace `app/page.tsx` with a minimal placeholder:

```tsx
export default function Home() {
  return <main />;
}
```

- Empty `app/globals.css` down to just the Tailwind import line that the template generated (keep `@import "tailwindcss";` or the v4 equivalent; remove everything else).
- In `app/layout.tsx`, remove template font/metadata cruft for now (real metadata comes in Task 4). Keep a valid root layout returning `<html><body>{children}</body></html>`.

- [ ] **Step 4: Verify it still builds and lints**

Run: `pnpm build && pnpm lint`
Expected: both succeed; no references to deleted SVGs.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore: scaffold Next.js app and strip create-next-app boilerplate"
```

---

### Task 2: Content data and site constants

**Files:**
- Create: `lib/constants.ts`
- Create: `lib/content.ts`

**Interfaces:**
- Consumes: nothing from prior tasks.
- Produces:
  - `lib/constants.ts` exports: `DOWNLOAD_URL: string`, `GITHUB_REPO_URL: string`, `FAQ_URL: string`, `SHA_VERIFY_CMD: string`.
  - `lib/content.ts` exports: `comparisonRows: ComparisonRow[]`, `installSteps: InstallStep[]`, `safetyPoints: SafetyPoint[]`, `agentResumeSnippet: string`, plus the exported types `ComparisonRow`, `InstallStep`, `SafetyPoint`.

- [ ] **Step 1: Write `lib/constants.ts`**

`DOWNLOAD_URL` is derived from `GITHUB_REPO_URL` so the link is correct by
construction — there is no separate string to drift.

```ts
export const GITHUB_REPO_URL =
  "https://github.com/elliot-ylambda/clinch-terminal";

export const DOWNLOAD_URL = `${GITHUB_REPO_URL}/releases/latest/download/Clinch.app.zip`;

export const FAQ_URL = `${GITHUB_REPO_URL}/blob/master/FAQ.md`;

export const SHA_VERIFY_CMD = "shasum -a 256 -c Clinch.app.zip.sha256";
```

- [ ] **Step 2: Write `lib/content.ts` (copy verbatim from README)**

```ts
export type ComparisonRow = { feature: string; clinch: string; warp: string };
export type InstallStep = { title: string; detail?: string; code?: string };
export type SafetyPoint = { title: string; body: string };

export const comparisonRows: ComparisonRow[] = [
  {
    feature: "Agent-session resume",
    clinch:
      "Reopens each tab and re-launches the Claude Code / Codex agent it was running",
    warp: "Restores the shell; the agent is gone",
  },
  {
    feature: "Sign-in",
    clinch: "None — fully local, never contacts Warp's servers",
    warp: "Account required",
  },
  {
    feature: "Warp AI, Drive, teams, session sharing",
    clinch: "Removed (can't run without Warp's backend)",
    warp: "Included",
  },
  { feature: "Platform", clinch: "macOS only", warp: "macOS / Linux / Windows" },
  {
    feature: "Bring your own CLI agent (Claude Code, Codex)",
    clinch: "Yes",
    warp: "Yes",
  },
];

export const installSteps: InstallStep[] = [
  {
    title: "Verify the download (recommended)",
    detail:
      "Each release ships a Clinch.app.zip.sha256 to confirm the bytes match.",
    code: "shasum -a 256 -c Clinch.app.zip.sha256",
  },
  { title: "Unzip and move Clinch.app to /Applications" },
  {
    title: "Clear the macOS quarantine flag once, then open it",
    detail:
      "Clinch is open source but not notarized, so macOS quarantines downloaded copies.",
    code: "xattr -dr com.apple.quarantine /Applications/Clinch.app",
  },
];

export const agentResumeSnippet = `git clone https://github.com/elliot-ylambda/clinch-terminal.git
cd clinch-terminal && ./tools/agent-resume/install.sh
# then restart your shell (or: source ~/.zshrc)`;

export const safetyPoints: SafetyPoint[] = [
  {
    title: "It's open source",
    body:
      "Every line is in the public repo under AGPL-3.0. The most trustworthy way to run Clinch is to build it yourself.",
  },
  {
    title: "Verify what you downloaded",
    body:
      "Each release publishes a SHA-256; shasum -a 256 -c Clinch.app.zip.sha256 confirms the bytes are exactly what's published.",
  },
  {
    title: "Why the xattr step?",
    body:
      "Apple's notarization requires a paid Developer account this project doesn't have. The app is code-signed — just not notarized — so Gatekeeper quarantines the download; the command clears that flag.",
  },
  {
    title: "install.sh is auditable",
    body:
      "The optional agent-resume installer only adds SessionStart hooks via a non-destructive jq merge and sources its replay functions from ~/.zshrc.",
  },
];

// --- Privacy & telemetry section (its own section, sharper differentiator) ---

export const lsofCheck = `lsof -nP -i -a -p "$(pgrep -x warp-oss | paste -sd, -)" | grep ESTABLISHED
# no output = no connections`;

export const privacyPoints: SafetyPoint[] = [
  {
    title: "No telemetry or analytics",
    body:
      "The build sets telemetry_config, crash_reporting_config, and autoupdate_config to None. No analytics write-keys or DSNs are baked in, and crash reporting (Sentry) isn't compiled into the binary at all.",
  },
  {
    title: "No backend, no sign-in",
    body:
      "Built with the skip_login feature: there's no login screen, and every authenticated request to Warp's servers hard-fails by design. It cannot phone home even if something tried.",
  },
  {
    title: "Verified at runtime",
    body:
      "While running, the warp-oss process holds zero outbound network connections. Check it yourself, or block *.warp.dev with a firewall rule and Clinch keeps working.",
  },
];

export const privacyCaveats: SafetyPoint[] = [
  {
    title: "Your CLI agents talk to their own providers",
    body:
      "Claude Code reaches Anthropic, Codex reaches OpenAI, MCP servers reach wherever you point them. That traffic is theirs, not Clinch's — the terminal only hosts them.",
  },
  {
    title: "One image-only exception",
    body:
      "A code path can fetch some static theme assets from Warp's asset server, with bundled fallbacks. It's a download, never a send, and runtime monitoring shows it inactive.",
  },
];
```

- [ ] **Step 3: Type-check the data**

Run: `pnpm build && pnpm lint`
Expected: both succeed (this compiles `lib/*` and catches any type mismatch in
the content arrays).

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: add site constants and landing content data"
```

---

### Task 3: Root layout, theme shell, Hero + Install (the functional core)

**Files:**
- Create: `components/download-button.tsx`
- Create: `components/terminal-window.tsx`
- Create: `components/hero.tsx`
- Create: `components/install.tsx`
- Modify: `app/globals.css` (terminal-flavored theme tokens), `app/page.tsx`

**Interfaces:**
- Consumes: `DOWNLOAD_URL`, `SHA_VERIFY_CMD` from `lib/constants`; `installSteps`, `agentResumeSnippet` from `lib/content`.
- Produces: `<Hero />`, `<Install />`, `<DownloadButton />` (default exports). `DownloadButton` renders an `<a href={DOWNLOAD_URL}>` with text "Download for macOS" — correctness is guaranteed by the constant, confirmed live in Task 7 Step 5.

- [ ] **Step 1: Implement `DownloadButton`**

```tsx
import { DOWNLOAD_URL } from "@/lib/constants";

export default function DownloadButton() {
  return (
    <a href={DOWNLOAD_URL} className="inline-flex items-center gap-2">
      Download for macOS
    </a>
  );
}
```

- [ ] **Step 2: Implement `TerminalWindow` (faux-terminal demo element)**

```tsx
export default function TerminalWindow({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-lg border font-mono text-sm">
      <div className="flex gap-1.5 px-3 py-2">
        <span className="h-3 w-3 rounded-full bg-red-400" />
        <span className="h-3 w-3 rounded-full bg-yellow-400" />
        <span className="h-3 w-3 rounded-full bg-green-400" />
      </div>
      <div className="px-4 py-3 whitespace-pre-wrap">{children}</div>
    </div>
  );
}
```

- [ ] **Step 3: Implement `Hero`**

```tsx
import DownloadButton from "./download-button";
import TerminalWindow from "./terminal-window";
import { GITHUB_REPO_URL } from "@/lib/constants";

export default function Hero() {
  return (
    <section>
      <p>macOS · open source · no sign-in</p>
      <h1>Clinch</h1>
      <p>
        A local-only fork of Warp that brings your CLI agents back when you
        reopen it.
      </p>
      <p>
        Quit with Claude Code or Codex running in your tabs, reopen, and each tab
        returns with its agent resumed (claude --resume / codex resume) — not a
        dead shell. No sign-in, no account, never phones home.
      </p>
      <div>
        <DownloadButton />
        <a href={GITHUB_REPO_URL}>View on GitHub</a>
      </div>
      <TerminalWindow>{"$ # tab restored\n$ claude --resume"}</TerminalWindow>
    </section>
  );
}
```

- [ ] **Step 4: Implement `Install`**

```tsx
import DownloadButton from "./download-button";
import { installSteps, agentResumeSnippet } from "@/lib/content";

export default function Install() {
  return (
    <section>
      <h2>Install</h2>
      <DownloadButton />
      <ol>
        {installSteps.map((step) => (
          <li key={step.title}>
            <strong>{step.title}</strong>
            {step.detail ? <p>{step.detail}</p> : null}
            {step.code ? <pre><code>{step.code}</code></pre> : null}
          </li>
        ))}
      </ol>
      <h3>Enable agent-session resume</h3>
      <pre><code>{agentResumeSnippet}</code></pre>
    </section>
  );
}
```

- [ ] **Step 5: Render Hero + Install in the page**

Replace `app/page.tsx`:

```tsx
import Hero from "@/components/hero";
import Install from "@/components/install";

export default function Home() {
  return (
    <main>
      <Hero />
      <Install />
    </main>
  );
}
```

- [ ] **Step 6: Apply the terminal-flavored visual design**

Use the `frontend-design:frontend-design` skill to style `globals.css` theme tokens and these components (dark background, monospace accents, the faux-terminal element) into a distinctive, non-templated look. Keep all `href`/text/content above unchanged.

- [ ] **Step 7: Build + lint**

Run: `pnpm build && pnpm lint`
Expected: both succeed.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat: add hero and install sections with macOS download button"
```

---

### Task 4: Remaining sections (what-it-does, comparison, is-this-safe, footer) + metadata

**Files:**
- Create: `components/what-it-does.tsx`
- Create: `components/comparison.tsx`
- Create: `components/is-this-safe.tsx`
- Create: `components/privacy-telemetry.tsx`
- Create: `components/site-footer.tsx`
- Create: `public/favicon.ico`, `public/og.png` (or `app/opengraph-image.png`)
- Modify: `app/page.tsx` (assemble all sections), `app/layout.tsx` (metadata)

**Interfaces:**
- Consumes: `comparisonRows`, `safetyPoints`, `privacyPoints`, `privacyCaveats`, `lsofCheck` from `lib/content`; `GITHUB_REPO_URL`, `FAQ_URL` from `lib/constants`.
- Produces: `<WhatItDoes />`, `<Comparison />`, `<IsThisSafe />`, `<PrivacyTelemetry />`, `<SiteFooter />` default exports; full page metadata.

- [ ] **Step 1: Implement `Comparison` (renders the table)**

```tsx
import { comparisonRows } from "@/lib/content";

export default function Comparison() {
  return (
    <section>
      <h2>How Clinch differs from Warp</h2>
      <table>
        <thead>
          <tr><th></th><th>Clinch</th><th>Warp</th></tr>
        </thead>
        <tbody>
          {comparisonRows.map((row) => (
            <tr key={row.feature}>
              <th scope="row">{row.feature}</th>
              <td>{row.clinch}</td>
              <td>{row.warp}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
```

- [ ] **Step 2: Implement `WhatItDoes`**

```tsx
export default function WhatItDoes() {
  return (
    <section>
      <h2>Your agents survive a restart</h2>
      <p>
        Clinch remembers which Claude Code or Codex session each tab was running.
        Quit and reopen, and every tab comes back with its agent resumed — not a
        fresh, empty shell. The only functional addition over Warp is
        agent-session resume; everything else is Warp with the login and cloud
        surfaces stripped out.
      </p>
    </section>
  );
}
```

- [ ] **Step 3: Implement `IsThisSafe`**

```tsx
import { safetyPoints } from "@/lib/content";

export default function IsThisSafe() {
  return (
    <section>
      <h2>Is this safe?</h2>
      <p>
        Fair question — you should be skeptical of any app that asks you to clear
        macOS quarantine. The honest picture:
      </p>
      <ul>
        {safetyPoints.map((point) => (
          <li key={point.title}>
            <strong>{point.title}.</strong> {point.body}
          </li>
        ))}
      </ul>
    </section>
  );
}
```

- [ ] **Step 4: Implement `PrivacyTelemetry`**

```tsx
import { privacyPoints, privacyCaveats, lsofCheck } from "@/lib/content";

export default function PrivacyTelemetry() {
  return (
    <section>
      <h2>Privacy & telemetry</h2>
      <p>
        Clinch sends no telemetry and makes zero calls to Warp's backend. This
        isn't a pinky-promise — it's how the build is compiled, and every claim
        is verifiable.
      </p>
      <ul>
        {privacyPoints.map((point) => (
          <li key={point.title}><strong>{point.title}.</strong> {point.body}</li>
        ))}
      </ul>
      <pre><code>{lsofCheck}</code></pre>
      <h3>What this does not cover (honestly)</h3>
      <ul>
        {privacyCaveats.map((point) => (
          <li key={point.title}><strong>{point.title}.</strong> {point.body}</li>
        ))}
      </ul>
      <p>Audit it or watch the wire — don't take our word for it.</p>
    </section>
  );
}
```

- [ ] **Step 5: Implement `SiteFooter`**

```tsx
import { GITHUB_REPO_URL, FAQ_URL } from "@/lib/constants";

export default function SiteFooter() {
  return (
    <footer>
      <nav>
        <a href={GITHUB_REPO_URL}>GitHub</a>
        <a href={FAQ_URL}>FAQ</a>
        <a href={`${GITHUB_REPO_URL}#build-from-source`}>Build from source</a>
      </nav>
      <p>
        Clinch is a modified version of warpdotdev/warp, licensed under AGPL-3.0.
        Not affiliated with Warp or Denver Technologies, Inc. "Warp" is their
        trademark; "Clinch" is an independent, unofficial fork.
      </p>
    </footer>
  );
}
```

- [ ] **Step 6: Assemble the full page**

Replace `app/page.tsx` (section order matches the spec: hero → what-it-does → comparison → install → is-this-safe → privacy → footer):

```tsx
import Hero from "@/components/hero";
import WhatItDoes from "@/components/what-it-does";
import Comparison from "@/components/comparison";
import Install from "@/components/install";
import IsThisSafe from "@/components/is-this-safe";
import PrivacyTelemetry from "@/components/privacy-telemetry";
import SiteFooter from "@/components/site-footer";

export default function Home() {
  return (
    <main>
      <Hero />
      <WhatItDoes />
      <Comparison />
      <Install />
      <IsThisSafe />
      <PrivacyTelemetry />
      <SiteFooter />
    </main>
  );
}
```

- [ ] **Step 7: Add metadata + OG image + favicon**

In `app/layout.tsx`, export metadata:

```tsx
import type { Metadata } from "next";

export const metadata: Metadata = {
  metadataBase: new URL("https://clinch.sh"),
  title: "Clinch — your CLI agents survive a restart",
  description:
    "A local-only macOS fork of Warp that brings your Claude Code / Codex agents back when you reopen it. No sign-in, never phones home.",
  openGraph: {
    title: "Clinch",
    description:
      "A local-only macOS fork of Warp that resumes your CLI agents on reopen.",
    url: "https://clinch.sh",
    images: ["/og.png"],
  },
};
```

Add `public/favicon.ico` and `public/og.png` (a simple terminal-styled card; can be produced during the frontend-design pass). If using `app/opengraph-image.png`, drop the explicit `images` entry.

- [ ] **Step 8: Apply visual design to the new sections**

Use `frontend-design:frontend-design` to style the comparison table, prose sections, privacy section, and footer consistently with Task 3. Keep text/links/structure intact.

- [ ] **Step 9: Build + lint**

Run: `pnpm build && pnpm lint`
Expected: both succeed.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "feat: add remaining landing sections, privacy, footer, and metadata"
```

---

### Task 5: Create the private GitHub repo and push

**Files:** none (git/infra).

**Interfaces:**
- Consumes: the committed local repo from Tasks 1–4.
- Produces: `elliot-ylambda/clinch-web` (private) on GitHub with `main` pushed.

- [ ] **Step 1: Confirm clean working tree and branch name**

```bash
cd /Users/ellioteckholm/projects/clinch-web
git status            # expect: clean
git branch -M main
```

- [ ] **Step 2: Create the private repo and push**

```bash
gh repo create elliot-ylambda/clinch-web --private --source=. --remote=origin --push
```

- [ ] **Step 3: Verify**

Run: `gh repo view elliot-ylambda/clinch-web --json visibility,defaultBranchRef -q '.visibility + " / " + .defaultBranchRef.name'`
Expected: `PRIVATE / main`

---

### Task 6: Link Vercel project, connect git, first production deploy

**Files:** none (infra). May create `.vercel/` (gitignored by default) and optionally `vercel.json`.

**Interfaces:**
- Consumes: the pushed repo.
- Produces: a Vercel project `clinch-web` under scope `elliot-ylambdas-projects`, connected to the GitHub repo, with a successful production deployment.

- [ ] **Step 1: Link the project to Vercel**

```bash
cd /Users/ellioteckholm/projects/clinch-web
vercel link --scope elliot-ylambdas-projects --yes --project clinch-web
```

If `vercel` is not authenticated, run `vercel login` first (USER authorizes in browser).

- [ ] **Step 2: Connect the GitHub repo for auto-deploy**

```bash
vercel git connect
```

This requires the Vercel GitHub app to have access to the private `clinch-web` repo. **USER ACTION:** when prompted (or via github.com/settings/installations), grant the Vercel app access to `elliot-ylambda/clinch-web`.

- [ ] **Step 3: First production deploy**

```bash
vercel --prod --yes
```

Note the deployment URL it prints (e.g. `https://clinch-web-xxxx.vercel.app`).

- [ ] **Step 4: Verify the deployment serves the site**

```bash
curl -sI "<deployment-url-from-step-3>" | head -1
```

Expected: `HTTP/2 200`. Optionally `curl -s <url> | grep -i "Download for macOS"` returns a match.

---

### Task 7: Attach the clinch.sh domain and verify the live site

**Files:** none (infra).

**Interfaces:**
- Consumes: the Vercel project `clinch-web` and the deployment from Task 6.
- Produces: `https://clinch.sh` serving the site, `www.clinch.sh` redirecting to apex.

- [ ] **Step 1: Inspect the exact domain subcommands (avoid guessing flags)**

```bash
vercel domains --help
```

`clinch.sh` already exists under the account, so this step attaches the existing domain to the `clinch-web` project rather than purchasing it.

- [ ] **Step 2: Attach apex + www to the project**

```bash
vercel domains add clinch.sh clinch-web --scope elliot-ylambdas-projects
vercel domains add www.clinch.sh clinch-web --scope elliot-ylambdas-projects
```

If the CLI form differs, assign both domains to `clinch-web` via the Vercel dashboard (Project → Settings → Domains), set `clinch.sh` as primary, and configure `www.clinch.sh` to **Redirect to clinch.sh**. Since DNS is Vercel-managed, no external DNS edits are needed.

- [ ] **Step 3: Verify the apex serves the site over HTTPS**

```bash
curl -sI https://clinch.sh | head -1
curl -s https://clinch.sh | grep -i "Download for macOS"
```

Expected: `HTTP/2 200`; grep matches.

- [ ] **Step 4: Verify the www redirect**

```bash
curl -sI https://www.clinch.sh | grep -iE "location|HTTP/"
```

Expected: a 30x with `location: https://clinch.sh/`.

- [ ] **Step 5: Verify the download button resolves to a real asset**

```bash
curl -sI "https://github.com/elliot-ylambda/clinch-terminal/releases/latest/download/Clinch.app.zip" | grep -iE "HTTP/|location"
```

Expected: a 302 redirect toward a `releases/download/<tag>/Clinch.app.zip` asset URL (confirms the static link is valid). If this does NOT redirect to an asset, STOP — there is no published release asset named `Clinch.app.zip` and the button would 404.

---

### Task 8: Cross-link the public clinch-terminal repo

**Files:**
- Modify: `clinch-terminal/README.md` (this repo, where the plan lives)

**Interfaces:**
- Consumes: the live `https://clinch.sh`.
- Produces: the public repo's homepage set to clinch.sh and a README link.

- [ ] **Step 1: Set the repo homepage**

```bash
gh repo edit elliot-ylambda/clinch-terminal --homepage "https://clinch.sh"
```

- [ ] **Step 2: Add a website link near the top of the README**

In `/Users/ellioteckholm/projects/clinch-terminal/README.md`, under the title line, add:

```markdown
**Website:** [clinch.sh](https://clinch.sh)
```

Place it right after the existing tagline paragraph. Do not touch any other lines (the repo has unrelated work in progress in `tools/agent-resume/`).

- [ ] **Step 3: Commit only the README change**

```bash
cd /Users/ellioteckholm/projects/clinch-terminal
git add README.md
git commit -m "docs: link the clinch.sh website from the README"
```

Leave the in-progress `tools/agent-resume/` changes unstaged.

- [ ] **Step 4: Verify**

```bash
gh repo view elliot-ylambda/clinch-terminal --json homepageUrl -q .homepageUrl
```

Expected: `https://clinch.sh`.

---

## Self-Review

**Spec coverage:**
- Private repo `clinch-web` → Task 5. ✓
- Next.js + TS + Tailwind v4 + pnpm → Task 1. ✓
- Landing-only, clean route tree for later `/docs` → Tasks 1–4 (no `/docs` created). ✓
- Seven sections (hero, what-it-does, comparison, install, is-this-safe, privacy & telemetry, footer) → Tasks 3–4. ✓
- Privacy & telemetry as its own section (compiled-out configs, skip_login, runtime `lsof` check, honest caveats, "audit it or watch the wire") → Task 2 (`privacyPoints`/`privacyCaveats`/`lsofCheck`) + Task 4 Step 4. ✓
- Download button = static latest-release URL → Pre-flight (asset resolves), Task 2 (constant, derived by construction), Task 3 (button), Task 7 Step 5 (live asset verify). ✓
- Vercel auto-deploy, scope, apex + www → Tasks 6–7. ✓
- Cross-link to public repo (homepage + README) → Task 8. ✓
- Strip boilerplate / no dead scaffolding → Task 1 Step 3. ✓
- Visual direction via frontend-design → Tasks 3 (Step 6), 4 (Step 8). ✓
- Attribution/license in footer → Task 4 Step 5 (`SiteFooter`). ✓
- YAGNI exclusions (no /docs, no installer, no analytics) → respected; none added. ✓

**Placeholder scan:** No "TBD/TODO". The only deferred-to-skill items (visual styling, OG image art) are explicitly scoped to the frontend-design pass with surrounding structure/content fully specified — not requirement placeholders.

**Type consistency:** `DOWNLOAD_URL`, `GITHUB_REPO_URL`, `FAQ_URL`, `SHA_VERIFY_CMD` defined in Task 2 and consumed consistently in Tasks 3–4. `ComparisonRow/InstallStep/SafetyPoint` and the arrays `comparisonRows/installSteps/safetyPoints/agentResumeSnippet/privacyPoints/privacyCaveats/lsofCheck` defined in Task 2 and consumed with matching field names (`feature/clinch/warp`, `title/detail/code`, `title/body`) in Tasks 3–4 (`privacyPoints`/`privacyCaveats` reuse the `SafetyPoint` `title/body` shape). Component default exports (`Hero/Install/DownloadButton/TerminalWindow/WhatItDoes/Comparison/IsThisSafe/PrivacyTelemetry/SiteFooter`) match their imports in `app/page.tsx`.

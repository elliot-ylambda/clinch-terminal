# Clinch.sh web app — landing page design

**Date:** 2026-06-30
**Status:** Approved (design); pending spec review
**Author:** Elliot + Claude

## Summary

Stand up `clinch.sh` as a single-page marketing/landing site for Clinch (the
local-only macOS Warp fork with agent-session resume). The site lives in a **new
private GitHub repo** `elliot-ylambda/clinch-web`, is built with **Next.js (App
Router) + TypeScript + Tailwind v4**, and is deployed on **Vercel** (scope
`elliot-ylambdas-projects`, which already owns the `clinch.sh` domain) with
auto-deploy on push to `main`.

At launch the site is **landing page only** — no `/docs` section — but the route
tree is kept clean so a `/docs` segment can be added later without restructuring.
Users install by clicking a **Download for macOS** button that links to the
latest GitHub release asset, then following copy-paste steps (the same flow the
product `README.md` documents today). No new installer tooling is built.

## Goals

- A live, public marketing page at `https://clinch.sh` (apex), `www` → apex.
- A primary install path: download the latest `Clinch.app.zip` + copy-paste setup.
- Cross-linking between the site and the public `clinch-terminal` repo.
- Private source repo; auto-deploy from GitHub via Vercel.

## Non-goals (YAGNI)

- No `/docs` content at launch (structure-only readiness for later).
- No `curl | sh` one-line installer (defer; download button is enough).
- No blog, CMS, auth, backend, database, or e-commerce.
- No analytics at launch (Vercel Analytics is a one-line add later if wanted).
- No shared-content pipeline between the repo `README.md` and the site copy.

## Decisions (locked during brainstorming)

| Decision | Choice |
|---|---|
| Repo | `elliot-ylambda/clinch-web` (private) |
| Stack | Next.js App Router + TypeScript + Tailwind v4, pnpm |
| Hosting | Vercel, scope `elliot-ylambdas-projects`, auto-deploy on push to `main` |
| Domain | `clinch.sh` apex (already registered via Vercel), `www` → apex redirect |
| Scope at launch | Landing page only; no `/docs` yet |
| Install UX | Download button + copy-paste steps (no new installer) |
| Execution | Claude runs `gh` + scaffold + push + Vercel link/domain; user authorizes Vercel/GitHub-app access when prompted |

## Architecture

### Repo & toolchain
- `create-next-app` with TypeScript + Tailwind + App Router; **pnpm** package
  manager (matches local toolchain: Node 23, pnpm 10).
- **`create-next-app` boilerplate is stripped** (default `page.tsx` content,
  sample SVGs in `public/`, starter globals) — no dead scaffolding left behind.
- Latest stable Next.js and Tailwind v4 at scaffold time; pinned in
  `package.json` after generation.

### Routes
- `/` — the single landing route.
- Route tree left clean (a future `app/docs/` segment can be added without moving
  `/`). No `/docs` shipped now.

### Deploy
- Vercel project `clinch-web` connected to the GitHub repo (git integration).
- Push to `main` → production deploy; PRs → preview deploys.
- `clinch.sh` (apex) assigned as the production domain; `www.clinch.sh` added and
  redirected to apex.

## The page (`/`)

Seven sections, copy **repurposed from the existing `README.md`** so messaging
stays consistent:

1. **Hero** — "Clinch"; one-liner ("brings your CLI agents back when you reopen
   it"); sub-line `macOS · open source · no sign-in`; primary **Download for
   macOS** CTA + secondary **View on GitHub**.
2. **What it does** — quit with Claude Code / Codex running → reopen → each tab
   returns with its agent resumed (`claude --resume` / `codex resume`), not a
   dead shell.
3. **Clinch vs Warp** — the comparison table from the README (agent-session
   resume, no sign-in, removed cloud features, macOS-only, BYO CLI agent).
4. **Install** — download button + the three copy-paste steps: unzip →
   `/Applications`; `xattr -dr com.apple.quarantine`; optional agent-resume
   hooks install. Includes the "Enable agent-session resume" block.
5. **Is this safe?** — the trust section: open source, verify SHA-256, why
   `xattr`, build-from-source, auditable `install.sh`.
6. **Privacy & telemetry** — the strongest differentiator, mirroring the README's
   [Privacy & telemetry](https://github.com/elliot-ylambda/clinch-terminal#privacy--telemetry)
   section: no telemetry/analytics (compiled out — `telemetry_config`/`crash_reporting_config`/`autoupdate_config`
   are `None`, no analytics keys baked in, Sentry not built), no backend/sign-in
   (`skip_login` hard-fails every authenticated request), and verified at runtime
   (the `warp-oss` process holds **zero** outbound connections — show the `lsof`
   one-liner and the "block `*.warp.dev` and it still works" check). Honest caveats:
   your CLI agents reach their own providers (that traffic is theirs, not Clinch's);
   one image-only theme-asset path to Warp's CDN with bundled fallbacks. Framing:
   "audit it or watch the wire — don't take our word for it."
7. **Footer** — AGPL-3.0 + "not affiliated with Warp / Denver Technologies"
   attribution; links to the GitHub repo, FAQ, and build-from-source.

### Download mechanics
The button is a **static link** to:

```
https://github.com/elliot-ylambda/clinch-terminal/releases/latest/download/Clinch.app.zip
```

GitHub redirects `…/releases/latest/download/<asset>` to the newest release's
asset — **no server code, no GitHub API call, no token**. (This is the exact URL
the product `README.md` already uses.) Optional later enhancement: a build-time
fetch of the latest tag to render a "vX.Y.Z" label; not in scope now.

### Visual direction
Distinctive, **terminal-flavored**: dark background, monospace accents, a small
faux-terminal element demonstrating the resume behavior — deliberately not a
generic SaaS template. Specifics locked at build time via the frontend-design
skill. Basic metadata, Open Graph image, and favicon included.

## Cross-linking the two repos

- The site links to the **public `clinch-terminal`** repo throughout (View on
  GitHub, Build from source, Is this safe?).
- The **only change to the existing public repo**: set its homepage to
  `https://clinch.sh` (`gh repo edit --homepage`) and add a one-line link in
  `README.md`. Done as a separate, minimal commit/PR on `clinch-terminal`.

## Risks / known tradeoffs

- **Copy duplication / drift.** Landing copy is duplicated from `README.md`; the
  two can diverge. Accepted for a 6-section page — sync manually and deliberately.
  A shared-content pipeline is explicitly out of scope.
- **Vercel ↔ private-repo access.** Vercel's GitHub app must be granted access to
  the new private repo for git integration. This is a one-time authorization the
  **user performs** when prompted. Fallback for the first deploy: `vercel deploy`
  from the CLI (no GitHub connection required) while the git integration is wired.
- **Download URL depends on a published release** named `Clinch.app.zip`. One
  already exists (the README links to it), so the static URL is valid today.

## Execution outline (full sequencing deferred to the implementation plan)

1. Scaffold Next.js app locally; strip boilerplate.
2. Build the single landing page (6 sections) + assets/metadata.
3. `gh repo create elliot-ylambda/clinch-web --private`; push.
4. `vercel link` the project under scope `elliot-ylambdas-projects`; connect the
   GitHub repo for auto-deploy; first production deploy.
5. Assign `clinch.sh` (apex) + `www` → apex redirect.
6. Cross-link: set `clinch-terminal` homepage to `https://clinch.sh` + README link.
7. Verify the live site and the download button resolve correctly.

## Spec location note

This spec is authored in the `clinch-terminal` repo (where the project's
`docs/superpowers/specs/` lives). It documents work that produces the separate
`clinch-web` repo; it can be copied into `clinch-web/docs/` after that repo exists.

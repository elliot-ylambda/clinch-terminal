# Launch Readiness Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix every must-fix from the 2026-07-09 pre-launch audit so Clinch can be publicly announced: de-Warp all user-visible surfaces, repoint community/security docs at this repo, make the README's privacy claims true for the shipped `stable` binary, publish a verifiable `.sha256` release asset, stamp real versions, default the Keychain-reading usage gauges off, harden `install.sh`, and clear the dependency advisories.

**Architecture:** Almost entirely string/doc/build-script changes; the only behavior changes are the usage-widget default flip, release-version stamping via the existing `GIT_RELEASE_TAG` compile-time hook, and a new `.sha256` release asset. Internal identifiers (action ids, enum names like `CustomAction::ShowAboutWarp`, fns like `join_slack`) are deliberately kept to minimize upstream-merge churn — only user-visible strings change. The entitlements fix (audit #5) already landed as `dfc6ddd0d`; it ships with the release this plan ends in.

**Tech Stack:** Rust (warpui custom UI framework), bash build scripts (`script/macos/bundle`, `script/update_plist`), Make, GitHub CLI, cargo-deny.

## Global Constraints

- Product-name copy rule: user-visible strings say **Clinch**. "Warp" may appear only nominatively ("Based on Warp…", "Warp Documentation (upstream)…", "Not affiliated with Warp").
- Internal identifiers stay unchanged: `workspace:quit_warp`, `workspace:join_slack`, `CustomAction::ShowAboutWarp`, `make_warp_default`, `WARP_*` env/script vars, `warp://cli-agent` OSC sentinel (per CLAUDE.md), bundle metadata for non-shipped channels (`warp-oss`, `preview`, `dev`, `warp`) except where a task names them.
- Repo URL: `https://github.com/elliot-ylambda/clinch-terminal`. Maintainer email: `contact@ylambda.com`. App bundle: `sh.clinch.Clinch`, binary/process name `stable`.
- Style (WARP.md): inline format args (`format!("{x}")`), no `_` wildcard match arms, remove unused imports/params entirely, `ctx` param last.
- Mutating `git`/`gh` commands must use absolute paths (`/usr/bin/git`, `/opt/homebrew/bin/gh`) — this machine strips PATH for approval-flagged Bash.
- Never run `gp` (hardcodes warpdotdev origin); push with plain `/usr/bin/git push clinch <branch>`.
- Multiple Claude sessions share this checkout — before starting, confirm `git status` is clean and work on branch `launch-readiness-fixes` (from up-to-date `main`).
- **Deliberate TDD deviation:** these are copy/doc/build-script swaps with no new logic; each task carries compile + `rg` + targeted-test verification instead of test-first. The one logic-adjacent change (settings default) runs its existing settings tests.
- One commit per task, message prefixes as given. Do not commit `Cargo.lock` changes except in Task 10.

---

### Task 0: Branch setup

**Files:** none (git only)

- [ ] **Step 1: Verify clean tree and sync**

Run:
```bash
/usr/bin/git -C /Users/ellioteckholm/projects/clinch-terminal status --short
/usr/bin/git -C /Users/ellioteckholm/projects/clinch-terminal fetch clinch main
/usr/bin/git -C /Users/ellioteckholm/projects/clinch-terminal log --oneline main..clinch/main
```
Expected: empty status; empty log (main up to date; as of plan-writing both are at `dfc6ddd0d`). If dirty or behind, STOP and reconcile with the other session first.

- [ ] **Step 2: Create the branch**

```bash
/usr/bin/git checkout -b launch-readiness-fixes main
```

---

### Task 1: Community & security docs point at Clinch

**Files:**
- Rewrite: `SECURITY.md`
- Modify: `CODE_OF_CONDUCT.md` (one line)
- Rewrite: `CONTRIBUTING.md`
- Delete: `FAQ.md`

**Interfaces:** Task 2's issue-template `config.yml` links to `SECURITY.md`'s advisory URL; Task 7's README keeps linking `CODE_OF_CONDUCT.md`/`SECURITY.md` paths unchanged.

- [ ] **Step 1: Replace SECURITY.md entirely with:**

```markdown
# Security Policy

Clinch is an independent fork of [warpdotdev/warp](https://github.com/warpdotdev/warp).
**Please do not report Clinch issues to Warp** — Warp does not maintain this project.

## Reporting a Vulnerability

If you believe you've found a security vulnerability, please follow responsible
disclosure practices and **do not** open a public GitHub issue or pull request.

Report it through one of these channels:

- **GitHub Security Advisory (preferred):** [Open a private advisory](https://github.com/elliot-ylambda/clinch-terminal/security/advisories/new)
- **Email:** [contact@ylambda.com](mailto:contact@ylambda.com)

This is a solo-maintained project; you'll get an acknowledgment as quickly as
possible, usually within a few days.

If the vulnerability also affects upstream Warp, please additionally report it
to Warp per [their security policy](https://github.com/warpdotdev/warp/blob/main/SECURITY.md).
```

- [ ] **Step 2: Fix the CODE_OF_CONDUCT.md enforcement contact (line 39)**

Replace:
```
...may be reported to the community leaders responsible for enforcement by emailing warp-coc at warp.dev. All complaints...
```
with:
```
...may be reported to the maintainer by emailing contact@ylambda.com. All complaints...
```

- [ ] **Step 3: Replace CONTRIBUTING.md entirely with:**

```markdown
# Contributing to Clinch

Clinch is a small, solo-maintained fork of
[warpdotdev/warp](https://github.com/warpdotdev/warp) focused on one thing:
resuming your CLI agent sessions (Claude Code, Codex) when the terminal
restarts. It is **not affiliated with Warp** — please don't take Clinch
questions to Warp's community channels or issue tracker.

## Bugs & feature requests

Open a [GitHub issue](https://github.com/elliot-ylambda/clinch-terminal/issues).
Include your Clinch version (**Settings → About**, or the release tag you
installed) and your macOS version.

For security vulnerabilities, see [SECURITY.md](SECURITY.md) — please don't
open a public issue.

## Pull requests

PRs are welcome and reviewed on a best-effort basis. Before opening one:

1. `./script/format`
2. `cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`
3. `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`

Architecture, build, and test guidance lives in [WARP.md](WARP.md); the
release flow is documented in [CLAUDE.md](CLAUDE.md).

## Code of conduct

The [Contributor Covenant](CODE_OF_CONDUCT.md) applies in all project spaces.
```

- [ ] **Step 4: Delete FAQ.md and confirm nothing links to it**

```bash
/usr/bin/git rm FAQ.md
rg -l "FAQ.md" --glob '!target' --glob '!docs/superpowers/plans'
```
Expected: `rg` returns nothing (the only pre-existing reference is an old plan doc, which is historical and stays).

- [ ] **Step 5: Commit**

```bash
/usr/bin/git add -A && /usr/bin/git commit -m "docs: route security/support/conduct contacts to Clinch, not Warp"
```

---

### Task 2: Issue templates + dependabot cleanup

**Files:**
- Rewrite: `.github/ISSUE_TEMPLATE/config.yml`
- Modify: `.github/ISSUE_TEMPLATE/01_bug_report.yml` (checkbox block)
- Modify: `.github/ISSUE_TEMPLATE/02_feature_request.yml` (2 labels + internal-label block)
- Delete: `.github/ISSUE_TEMPLATE/03_ssh_tmux.yml`, `.github/ISSUE_TEMPLATE/04_ssh_legacy.yml`, `.github/dependabot.yml`

- [ ] **Step 1: Replace `.github/ISSUE_TEMPLATE/config.yml` entirely with:**

```yaml
blank_issues_enabled: true
contact_links:
  - name: Clinch Releases
    url: https://github.com/elliot-ylambda/clinch-terminal/releases
    about: Download the latest build and read release notes.
  - name: clinch.sh
    url: https://clinch.sh
    about: Install instructions and project overview.
  - name: Report a security vulnerability
    url: https://github.com/elliot-ylambda/clinch-terminal/security/advisories/new
    about: Private disclosure — please don't open a public issue for security bugs.
  - name: Upstream Warp issues
    url: https://github.com/warpdotdev/warp/issues
    about: Only for bugs that reproduce in official Warp itself — Clinch is not affiliated with Warp.
```

- [ ] **Step 2: Fix `01_bug_report.yml`**

Replace the four-checkbox `Pre-submit Checks` block (lines 5–16) with:

```yaml
  - type: checkboxes
    attributes:
      label: "Pre-submit Checks"
      options:
        - label: "I have [searched Clinch issues](https://github.com/elliot-ylambda/clinch-terminal/issues) and there are no duplicates"
          required: true
        - label: "This reproduces in Clinch (if it also reproduces in official Warp, consider filing [upstream](https://github.com/warpdotdev/warp/issues) too)"
          required: false
```

Also scan the rest of the file for `warpdotdev` / `docs.warp.dev` links and remove or repoint them to this repo:
```bash
rg -n "warpdotdev|warp.dev" .github/ISSUE_TEMPLATE/01_bug_report.yml
```
Expected after edits: no output.

- [ ] **Step 3: Fix `02_feature_request.yml`**

Line 9: replace with
```yaml
        - label: "I have [searched Clinch issues](https://github.com/elliot-ylambda/clinch-terminal/issues?q=is%3Aissue) and there are no duplicates"
```
Line 11: replace with
```yaml
        - label: "This is a request for Clinch (Warp product requests belong [upstream](https://github.com/warpdotdev/warp/issues))"
```
Delete the checkbox item whose label starts with `"Warp Internal (ignore) - linear-label:…"` (line ~64) — it drives Warp's internal Linear triage and is meaningless here. Verify:
```bash
rg -n "warpdotdev|warp.dev|linear-label" .github/ISSUE_TEMPLATE/02_feature_request.yml
```
Expected: only the intentional upstream link from the new line 11.

- [ ] **Step 4: Delete Warp-support-specific templates and dependabot config**

```bash
/usr/bin/git rm .github/ISSUE_TEMPLATE/03_ssh_tmux.yml .github/ISSUE_TEMPLATE/04_ssh_legacy.yml .github/dependabot.yml
```
(dependabot only bumps the intentionally-unused upstream GitHub Actions workflows and assigns `warpdotdev/tech-leads` as reviewers — pure noise for this fork.)

- [ ] **Step 5: Close the open dependabot PRs**

```bash
for pr in 14 15 16 17 23; do /opt/homebrew/bin/gh pr close "$pr" --repo elliot-ylambda/clinch-terminal --comment "Dependabot config removed — these bump the intentionally-unused upstream CI workflows."; done
```

- [ ] **Step 6: Commit**

```bash
/usr/bin/git add -A && /usr/bin/git commit -m "docs(github): Clinch-native issue templates; drop dependabot for unused CI"
```

---

### Task 3: In-app rebrand — menus, titles, dialogs

**Files:**
- Modify: `crates/warpui/src/platform/mac/menus.rs:218,223`
- Modify: `app/src/app_menus.rs:231,273,329`
- Modify: `app/src/workspace/mod.rs:965,1470`
- Modify: `app/src/root_view.rs:111,706,749,801,1173,1358`
- Modify: `app/src/quit_warning/mod.rs:434`
- Modify: `app/src/resource_center/utils.rs:127,137`
- Modify: `app/src/workspace/cli_install.rs:75,111`

**Interfaces:** Display strings only; every action id, selector, and function name is unchanged, so no call sites move.

- [ ] **Step 1: Apply the string replacements**

| File:line | Old string | New string |
|---|---|---|
| `crates/warpui/src/platform/mac/menus.rs:218` | `ns_string!("Quit Warp")` | `ns_string!("Quit Clinch")` |
| `crates/warpui/src/platform/mac/menus.rs:223` | `ns_string!("Hide Warp")` | `ns_string!("Hide Clinch")` |
| `app/src/app_menus.rs:231` | `"Set Warp as Default Terminal"` | `"Set Clinch as Default Terminal"` |
| `app/src/app_menus.rs:273` | `Menu::new("Warp", menu_items)` | `Menu::new("Clinch", menu_items)` |
| `app/src/app_menus.rs:329` | `"Use Warp's Prompt"` | `"Use Clinch's Prompt"` |
| `app/src/workspace/mod.rs:965` | `"Quit Warp",` (EditableBinding desc) | `"Quit Clinch",` |
| `app/src/workspace/mod.rs:1470` | `.with_custom_description(bindings::MAC_MENUS_CONTEXT, "About Warp")` | `.with_custom_description(bindings::MAC_MENUS_CONTEXT, "About Clinch")` |
| `app/src/root_view.rs:111` | `const WINDOW_TITLE: &str = "Warp";` | `const WINDOW_TITLE: &str = "Clinch";` |
| `app/src/root_view.rs:706,749,801,1173,1358` | `title: Some("Warp".to_owned()),` | `title: Some("Clinch".to_owned()),` |
| `app/src/quit_warning/mod.rs:434` | `QuitScope::App => "Quit Warp?",` | `QuitScope::App => "Quit Clinch?",` |
| `app/src/resource_center/utils.rs:127` | `"Hide Warp".into(),` | `"Hide Clinch".into(),` |
| `app/src/resource_center/utils.rs:137` | `"Quit Warp".into(),` | `"Quit Clinch".into(),` |
| `app/src/workspace/cli_install.rs:75` | `…with prompt \"Warp needs administrator privileges to install the command in /usr/local/bin.\"…` | `…with prompt \"Clinch needs administrator privileges to install the command in /usr/local/bin.\"…` |
| `app/src/workspace/cli_install.rs:111` | `…with prompt \"Warp needs administrator privileges to uninstall the command from /usr/local/bin.\"…` | `…with prompt \"Clinch needs administrator privileges to uninstall the command from /usr/local/bin.\"…` |

- [ ] **Step 2: Verify no other reachable app-menu "Warp" strings remain**

```bash
rg -n '"(Quit|Hide|About) Warp|"Warp"' app/src/app_menus.rs app/src/root_view.rs app/src/workspace/mod.rs app/src/quit_warning app/src/resource_center crates/warpui/src/platform/mac/menus.rs
```
Expected: no output.

- [ ] **Step 3: Compile check**

```bash
cargo check -p warp -p warpui 2>&1 | tail -5
```
Expected: `Finished` with no errors.

- [ ] **Step 4: Commit**

```bash
/usr/bin/git add -A && /usr/bin/git commit -m "fix(branding): user-visible menus, titles, and dialogs say Clinch"
```

---

### Task 4: Help/feedback links → Clinch repo

**Files:**
- Modify: `app/src/util/links.rs` (full replacement below)
- Modify: `app/src/app_menus.rs:989-998` (`make_new_help_menu`)
- Modify: `app/src/resource_center/view.rs:34-58,251-263,405-460` (Slack → GitHub)
- Modify: `app/src/workspace/view.rs:6427,9625` and `app/src/workspace/mod.rs:1587`

**Interfaces:**
- Produces: `links::COMMUNITY_URL: &str` (replaces `links::SLACK_URL`); `links::GITHUB_ISSUES_URL` and `links::feedback_form_url()` keep their signatures but point at this repo. `ResourceCenterFooterItem::GitHub` replaces `::Slack`.

- [ ] **Step 1: Replace `app/src/util/links.rs` entirely with:**

```rust
use crate::channel::ChannelState;

/// Upstream Warp's docs still accurately describe the terminal features
/// Clinch inherits; linked nominatively ("Warp Documentation (upstream)").
pub const USER_DOCS_URL: &str = "https://docs.warp.dev/";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const GITHUB_ISSUES_URL: &str = "https://github.com/elliot-ylambda/clinch-terminal/issues";
pub const COMMUNITY_URL: &str = "https://github.com/elliot-ylambda/clinch-terminal";
pub const PRIVACY_POLICY_URL: &str =
    "https://github.com/elliot-ylambda/clinch-terminal#privacy--telemetry";

pub fn feedback_form_url() -> String {
    let mut url =
        url::Url::parse("https://github.com/elliot-ylambda/clinch-terminal/issues/new/choose")
            .expect("Should not fail to parse");
    if let Some(version) = ChannelState::app_version() {
        url.query_pairs_mut().append_pair("clinch-version", version);
    }
    url.query_pairs_mut()
        .append_pair("os-version", &os_info::get().version().to_string());
    url.to_string()
}
```

- [ ] **Step 2: Rewrite the Help menu (`app/src/app_menus.rs:989-998`)**

```rust
fn make_new_help_menu() -> Menu {
    Menu::new(
        "Help",
        vec![
            feedback_menu_item(),
            link_menu_item("Clinch on GitHub...", links::COMMUNITY_URL.into()),
            link_menu_item("GitHub Issues...", links::GITHUB_ISSUES_URL.into()),
            link_menu_item("Warp Documentation (upstream)...", links::USER_DOCS_URL.into()),
        ],
    )
}
```
(The "Warp Slack Community..." item is removed — do not funnel Clinch users into Warp's Slack.)

- [ ] **Step 3: Resource center footer: Slack → GitHub (`app/src/resource_center/view.rs`)**

- Line 35: `const SLACK_SVG_PATH: &str = "bundled/svg/slack-logo.svg";` → `const GITHUB_SVG_PATH: &str = "bundled/svg/github.svg";`
- Rename the enum variant `ResourceCenterFooterItem::Slack` → `ResourceCenterFooterItem::GitHub` and update its match arms (label `"Slack"` → `"GitHub"`, svg path const, click handler `ctx.open_url(links::SLACK_URL)` → `ctx.open_url(links::COMMUNITY_URL)`).
- Rename the mouse-state field `join_slack` → `open_github` in the button-mouse-states struct and its use at line ~414.
- Rename the local `slack_button` → `github_button` in `render_footer` (line ~451) and its `with_child` use.

Verify all sites converted:
```bash
rg -n "Slack|SLACK" app/src/resource_center/view.rs
```
Expected: no output.

- [ ] **Step 4: Workspace strings**

- `app/src/workspace/view.rs:6427` (`fn join_slack`): body becomes `ctx.open_url(links::COMMUNITY_URL);` (keep the fn name — it's wired to the `workspace:join_slack` action id).
- `app/src/workspace/view.rs:9625`: `MenuItemFields::new("Slack")` → `MenuItemFields::new("GitHub")`.
- `app/src/workspace/mod.rs:1587`: `"Join our Slack community (opens external link)"` → `"Open Clinch on GitHub (opens external link)"`.

- [ ] **Step 5: Verify nothing still references the removed const, then compile**

```bash
rg -n "SLACK_URL" app/src crates
cargo check -p warp 2>&1 | tail -5
```
Expected: no `SLACK_URL` references; clean check.

- [ ] **Step 6: Commit**

```bash
/usr/bin/git add -A && /usr/bin/git commit -m "fix(links): help/feedback/community links point at Clinch repo, not Warp properties"
```

---

### Task 5: About page rebrand

**Files:**
- Create: `app/assets/bundled/svg/clinch-logo.svg` (copy of existing icon source)
- Modify: `app/src/settings_view/about_page.rs`

**Interfaces:** Consumes `ChannelState::app_version()` (already used) — Task 6 makes it return the real release tag.

- [ ] **Step 1: Add the bundled Clinch logo asset**

```bash
cp app/channels/stable/icon/clinch-icon-source.svg app/assets/bundled/svg/clinch-logo.svg
```

- [ ] **Step 2: Rewrite the widget render (`about_page.rs`)**

Replace the theme-branched `image_path` block (lines 66–70) with:
```rust
        let image_path = "bundled/svg/clinch-logo.svg";
```
Remove the now-unused `ColorScheme` import (`use crate::themes::theme::ColorScheme;`) if nothing else in the file uses it (verify with `rg -n "ColorScheme" app/src/settings_view/about_page.rs`).

Constrain the logo square (it's an icon, not a wordmark) — change the `ConstrainedBox` maxima (lines 111–112) to:
```rust
                    .with_max_height(100.)
                    .with_max_width(100.)
```

Add a product-name span directly after the image child and replace the copyright span (line 116–122) so the column reads image → name → version row → attribution:
```rust
                .with_child(
                    ui_builder
                        .span("Clinch")
                        .build()
                        .with_margin_top(12.)
                        .finish(),
                )
                .with_child(version_row.finish())
                .with_child(
                    ui_builder
                        .span("© 2026 Clinch contributors — AGPL-3.0")
                        .build()
                        .with_margin_top(16.)
                        .finish(),
                )
                .with_child(
                    ui_builder
                        .span("Based on Warp © Denver Technologies, Inc. Not affiliated with Warp.")
                        .with_soft_wrap()
                        .build()
                        .with_margin_top(4.)
                        .finish(),
                )
```
(Keep the existing `version_row` construction unchanged; it moves between the new name span and the attribution spans.)

Update `search_terms` (line 54): `"about warp version"` → `"about clinch warp version license"`.

- [ ] **Step 3: Compile and visually spot-check**

```bash
cargo check -p warp 2>&1 | tail -3
```
Expected: clean. Full visual check happens in Task 11's smoke test (Settings → About shows Clinch icon, name, version tag, dual attribution).

- [ ] **Step 4: Commit**

```bash
/usr/bin/git add -A && /usr/bin/git commit -m "fix(about): Clinch logo, name, and AGPL attribution replace Warp wordmark"
```

---

### Task 6: Bundle metadata — copyright, TCC strings, DMG, version stamping, sha256 asset

**Files:**
- Modify: `app/Cargo.toml:1034` (stable copyright), `app/assets/resources/mac/CLI-Info.plist:22`
- Modify: `script/update_plist` (TCC strings + version stamp)
- Modify: `script/macos/bundle:760` (drop DMG background)
- Delete: `app/assets/resources/mac/warp_install_image.png`
- Modify: `Makefile` (GIT_RELEASE_TAG + sha256 asset)
- Modify: `CLAUDE.md` (remove now-stale follow-up bullet)

**Interfaces:** Produces release assets `Clinch.dmg`, `Clinch.app.zip`, `Clinch.app.zip.sha256`; Task 8's README references the `.sha256` file. `GIT_RELEASE_TAG` feeds `ChannelState::app_version()` (About page) and `CFBundleShortVersionString` (Finder Get Info).

- [ ] **Step 1: Copyright strings**

`app/Cargo.toml` `[package.metadata.bundle.bin.stable]` (line 1034):
```toml
copyright = "© 2026 Clinch contributors. Based on Warp © Denver Technologies, Inc. Not affiliated with Warp. AGPL-3.0."
```
`app/assets/resources/mac/CLI-Info.plist` (line 22): same string inside `<string>…</string>`.
Leave the other channels' copyright lines untouched (not shipped).

- [ ] **Step 2: Parameterize the TCC permission strings in `script/update_plist`**

Directly above the `echo "Updating plist with permissions descriptions"` line, insert:
```bash
# Use the bundle's real display name so consent dialogs don't say "Warp" in a
# Clinch build (Info.plist CFBundleName is authoritative for every channel).
APP_DISPLAY_NAME="$(/usr/libexec/PlistBuddy -c 'Print CFBundleName' "$WARP_PLIST_PATH" 2>/dev/null || echo "Warp")"
```
Then in the seven `plutil -insert NS...UsageDescription` lines, replace every literal `Warp` with `$APP_DISPLAY_NAME`, e.g.:
```bash
plutil -insert NSAppleEventsUsageDescription -string "A program in $APP_DISPLAY_NAME wants to use AppleScript." "$WARP_PLIST_PATH"
```
(same pattern for Camera, Microphone, Contacts, Calendars, Location, PhotoLibrary).

- [ ] **Step 3: Stamp the release version in `script/update_plist`**

At the end of the file append:
```bash
if [[ -n "${GIT_RELEASE_TAG:-}" ]]; then
  echo "Stamping CFBundleShortVersionString=${GIT_RELEASE_TAG#v}"
  plutil -replace CFBundleShortVersionString -string "${GIT_RELEASE_TAG#v}" "$WARP_PLIST_PATH"
fi
```

- [ ] **Step 4: Drop the Warp DMG background**

In `script/macos/bundle` remove the line:
```bash
      --background app/assets/resources/mac/warp_install_image.png
```
then:
```bash
/usr/bin/git rm app/assets/resources/mac/warp_install_image.png
rg -ln "warp_install_image"
```
Expected: `rg` returns nothing. (The Makefile already sets `SKIP_DMG_APPLESCRIPT=1`, so the background was skipped in practice; this makes it true in all invocations and deletes the "GET WARPING" art.)

- [ ] **Step 5: Makefile — version stamp + sha256 asset**

In the `release:` recipe, change the bundle line to export the tag:
```make
	GIT_RELEASE_TAG="$(VERSION)" ./script/bundle -c stable --selfsign $(BUNDLE_ARCH_FLAG)
```
Add below the `RELEASE_ZIP` definition:
```make
RELEASE_SHA        := $(RELEASE_ZIP).sha256
```
After the `ditto -c -k --keepParent …` line add:
```make
	cd target/$(STABLE_PROFILE_DIR)/bundle/osx && shasum -a 256 "$(STABLE_APP).app.zip" > "$(STABLE_APP).app.zip.sha256"
```
And extend the `gh release create` line's assets:
```make
	gh release create "$(VERSION)" "$(RELEASE_DMG)" "$(RELEASE_ZIP)" "$(RELEASE_SHA)" \
```
(Note: `GIT_RELEASE_TAG` is read by `option_env!` in `crates/warp_core/src/channel/state.rs:347`, so cargo rebuilds `warp_core` and dependents each release — expected, release builds are cold anyway.)

- [ ] **Step 6: Remove the stale CLAUDE.md follow-up**

In `CLAUDE.md` under "Other follow-ups (not done)", delete the bullet:
```
- The copyright string in the bundle metadata is still Warp's entity.
```

- [ ] **Step 7: Syntax-check the scripts**

```bash
bash -n script/update_plist && bash -n script/macos/bundle && make -n release SKIP_SYNC=1 >/dev/null && echo OK
```
Expected: `OK`.

- [ ] **Step 8: Commit**

```bash
/usr/bin/git add -A && /usr/bin/git commit -m "fix(bundle): Clinch copyright/TCC strings, drop Warp DMG art, stamp version, ship sha256"
```

---

### Task 7: Usage plan-limit gauges default off

**Files:**
- Modify: `app/src/settings/cli_agent_usage.rs`

**Interfaces:** Consumed by `app/src/ai/blocklist/usage/cli_agent_usage_model.rs`, `app/src/settings_view/features_page.rs`, `app/src/workspace/view.rs` via the generated `ShowCliAgentPlanLimits` setting — no signature changes, only the default. **Must land before Task 8**, whose README copy states the gauges are off by default.

- [ ] **Step 1: Flip the default and document it**

In `define_settings_group!` change:
```rust
        default: true,
```
to:
```rust
        default: false,
```
and update the doc comment's first sentence to note the launch posture:
```rust
    // Gates the Claude Code live plan-limit gauges (the 5-hour and weekly
    // rate-limit % in the tab-bar usage widget). Off by default: populating
    // them requires reading Claude Code's OAuth token from the macOS Keychain
    // (a password prompt) and querying Anthropic's usage endpoint — both are
    // opt-in so a fresh install never touches the Keychain or the network.
```
Update the user-facing `description:` string to match:
```rust
        description: "Show Claude Code's live plan-limit gauges in the usage \
                      widget. When enabled, reads the 'Claude Code-credentials' \
                      item from your macOS Keychain (asks for your password) and \
                      queries Anthropic's usage endpoint. Off by default; local \
                      token and cost stats work without it.",
```

- [ ] **Step 2: Run the tests that touch this setting**

```bash
cargo nextest run -p warp -E 'test(/cli_agent_usage/)' --no-fail-fast
```
Expected: PASS (fix any test asserting the old default by updating the expectation to `false` — the default flip is the intended behavior change).

- [ ] **Step 3: Commit**

```bash
/usr/bin/git add -A && /usr/bin/git commit -m "fix(usage): plan-limit gauges opt-in — no Keychain read or Anthropic call by default"
```

> Post-ship note for the maintainer: re-enable locally via Settings → AI → Show plan limits (or `ai.cli_agent_usage.show_plan_limits = true`).

---

### Task 8: README truth pass

**Files:**
- Modify: `README.md` (Download step 1, "Is this safe?" bullet 2, entire "Privacy & telemetry" section, Build-from-source note)

**Interfaces:** Depends on Task 6 (the `.sha256` asset exists) and Task 7 (gauges default off) — both claims below must already be true in this branch.

- [ ] **Step 1: Download section, step 1 (lines 15–18) — becomes true once Task 6 ships; clarify the GitHub digest too:**

```markdown
1. **(Recommended) Verify the download.** Each release attaches a `Clinch.app.zip.sha256` (the same digest GitHub shows next to the asset on the release page):
   ```bash
   shasum -a 256 -c Clinch.app.zip.sha256
   ```
```

- [ ] **Step 2: "Is this safe?" bullet 2 (line 43):**

```markdown
- **Verify what you downloaded.** Each release attaches a `Clinch.app.zip.sha256` file, and GitHub displays the same SHA-256 digest on the release page; `shasum -a 256 -c Clinch.app.zip.sha256` confirms the bytes are exactly what's published here.
```

- [ ] **Step 3: Replace the three claim bullets of "Privacy & telemetry" (lines 52–59) with:**

```markdown
- **No telemetry or analytics.** The released app is the `stable` binary, built from [`app/src/bin/stable.rs`](app/src/bin/stable.rs) with [`ChannelConfig::no_backend()`](crates/warp_core/src/channel/config.rs), which sets `telemetry_config`, `crash_reporting_config`, and `autoupdate_config` to `None`. No analytics write-keys or DSNs are baked in, and crash reporting (Sentry) isn't compiled into the binary at all. The telemetry code that exists upstream has no destination to send to and is gated off.
- **No backend, no sign-in.** `no_backend()` reports `has_backend() == false` — the login and cloud surfaces never initialize — and points every server URL at `http://192.0.2.0:9`, a reserved, unroutable test address. Clinch cannot reach Warp's servers even if something tried.
- **Verified at runtime.** The installed app's process is named `stable` (inside `Clinch.app`). While running, it holds zero outbound connections of its own:
  ```bash
  lsof -nP -i -a -p "$(pgrep -f 'Clinch.app/Contents/MacOS/stable' | paste -sd, -)" | grep ESTABLISHED
  # no output = no connections. (If you enable the optional plan-limit gauges,
  # you may see one connection to api.anthropic.com — see below.)
  ```
  Or just block it: add a firewall / Little Snitch rule denying `*.warp.dev`, and Clinch keeps working — because it needs nothing from them.
```

- [ ] **Step 4: Add the usage-widget disclosure to "What this does _not_ cover" (after the CLI-agents bullet, before the image-only bullet):**

```markdown
- **The optional plan-limit gauges query Anthropic — off by default.** If you turn on **Settings → AI → Show plan limits**, Clinch reads Claude Code's OAuth token from your macOS Keychain (macOS asks for your permission first) and calls `https://api.anthropic.com/api/oauth/usage` to show your own rate-limit usage in the tab bar. The token goes only to Anthropic — the same host Claude Code itself sends it to — and nowhere else. Leave the setting off and Clinch never touches your Keychain or Anthropic; the local cost/token stats still work by scanning `~/.claude` files.
```

- [ ] **Step 5: Build-from-source honesty note — append to the paragraph after the code block (line 88):**

```markdown
`build-app.sh` builds the `warp-oss` binary variant (compiled with the `skip_login` hard-fail); the *distributed* releases are the `stable` binary built via `./script/bundle -c stable --selfsign`, which is what the privacy section above describes. Both are no-backend builds.
```

- [ ] **Step 6: Verify no stale claims remain**

```bash
rg -n "warp-oss|skip_login|oss.rs" README.md
```
Expected: hits only inside the Build-from-source note added in Step 5.

- [ ] **Step 7: Commit**

```bash
/usr/bin/git add README.md && /usr/bin/git commit -m "docs(readme): privacy claims describe the shipped stable binary; disclose usage gauges"
```

---

### Task 9: install.sh partial-download hardening

**Files:**
- Modify: `install.sh`

- [ ] **Step 1: Wrap execution in `main()`**

Keep the shebang, comment header, `set -eu`, the constants (`REPO`/`APP_NAME`/`ASSET`/`DOWNLOAD_URL`), and the `say`/`fail` definitions at top level. Then wrap **everything from the Darwin check (line 35) through `open "$DEST"` (line 110)** in:
```sh
main() {
    # …existing lines 35–110, unchanged, indented one level…
}

main "$@"
```
`main "$@"` must be the final line. A truncated `curl | sh` stream then defines an incomplete function and executes nothing, instead of running half a script.

- [ ] **Step 2: Syntax check + dry-run the parse**

```bash
sh -n install.sh && echo SYNTAX-OK
```
Expected: `SYNTAX-OK`. (Full end-to-end install re-verified in Task 11 against the new release.)

- [ ] **Step 3: Commit**

```bash
/usr/bin/git add install.sh && /usr/bin/git commit -m "fix(install): wrap installer in main() so truncated downloads execute nothing"
```

---

### Task 10: Dependency advisory cleanup (can trail the announcement if time is short)

**Files:**
- Modify: `Cargo.lock` (via cargo update), `deny.toml`

- [ ] **Step 1: Apply the semver-compatible fixes**

```bash
cargo update -p anyhow -p crossbeam-epoch -p diesel -p memmap2
```
Expected: anyhow ≥1.0.103, crossbeam-epoch ≥0.9.20, diesel ≥2.3.10, memmap2 ≥0.9.11 in `Cargo.lock`.

- [ ] **Step 2: Confirm git2/quick-xml need semver-major bumps (not today)**

```bash
cargo tree -i git2 --depth 1 | head -5
cargo tree -i quick-xml --depth 1 | head -5
```
Both fixes (git2 0.21, quick-xml 0.41) are 0.x-major jumps; git2 is a direct-API dependency and quick-xml is transitive — defer both.

- [ ] **Step 3: Record the deferred advisories in `deny.toml`**

Open `deny.toml`; in the `[advisories]` section (create it if absent, or append to an existing `ignore` array):
```toml
[advisories]
ignore = [
    # Deferred until post-launch dependency work — all four need 0.x semver-major bumps:
    "RUSTSEC-2026-0183", # git2: Remote::list() UB — fixed in 0.21 (API-breaking bump)
    "RUSTSEC-2026-0184", # git2: BlameHunk Signature UB — same bump
    "RUSTSEC-2026-0194", # quick-xml: quadratic dup-attribute check — transitive, needs parent bump
    "RUSTSEC-2026-0195", # quick-xml: NsReader memory-exhaustion DoS — same
]
```

- [ ] **Step 4: Verify the audit is green and the workspace still builds**

```bash
cargo deny check advisories 2>&1 | tail -3
cargo check -p warp 2>&1 | tail -3
```
Expected: `advisories ok`; clean check.

- [ ] **Step 5: Commit**

```bash
/usr/bin/git add Cargo.lock deny.toml && /usr/bin/git commit -m "chore(deps): apply compatible RUSTSEC fixes; document deferred git2/quick-xml bumps"
```

---

### Task 11: Verify, ship, and smoke-test

**Files:** none new (verification + release)

- [ ] **Step 1: Repo-standard checks (required before any push per WARP.md)**

```bash
./script/format
cargo clippy --workspace --all-targets --all-features --tests -- -D warnings 2>&1 | tail -5
cargo nextest run --no-fail-fast -p warp -p warpui -p cli_agent_usage 2>&1 | tail -10
```
Expected: format clean, clippy clean, tests PASS. If time allows before announcing, run full `./script/presubmit` instead of the targeted nextest.

- [ ] **Step 2: Push, PR, merge**

```bash
/usr/bin/git push clinch launch-readiness-fixes
/opt/homebrew/bin/gh pr create --repo elliot-ylambda/clinch-terminal --base main --title "Launch readiness: de-Warp UI/docs, truthful privacy claims, sha256 asset, opt-in usage gauges" --fill
```
Review the diff, then merge and sync local main:
```bash
/opt/homebrew/bin/gh pr merge --repo elliot-ylambda/clinch-terminal --squash --delete-branch
/usr/bin/git checkout main && /usr/bin/git pull clinch main
```

- [ ] **Step 3: Cut the release and update this machine**

```bash
make update
```
(Builds stable with `GIT_RELEASE_TAG`, publishes DMG + zip + **zip.sha256**, swaps `/Applications/Clinch.app`, relaunches.)

- [ ] **Step 4: Launch-day smoke checklist (in the relaunched app + fresh eyes)**

- [ ] App menu reads **Clinch** — "Quit Clinch" (⌘Q), "Hide Clinch" (⌘H), "About Clinch".
- [ ] Settings → About: Clinch icon, name "Clinch", the real release tag (not `v#.##.###` or 0.1.0), AGPL attribution line; Finder Get Info shows the new copyright + version.
- [ ] ⌘Q shows "Quit Clinch?".
- [ ] Help menu: Clinch on GitHub / GitHub Issues / Warp Documentation (upstream) — no Slack entry; each link opens the right page.
- [ ] Fresh-launch: **no Keychain password prompt** (gauges now opt-in). Enable Show plan limits → prompt appears once → "Always Allow" persists across relaunch (entitlements fix `dfc6ddd0d`).
- [ ] Release page: three assets; `shasum -a 256 -c Clinch.app.zip.sha256` passes against the downloaded zip.
- [ ] `curl -fsSL https://clinch.sh/install | sh` end-to-end on the new release (quit Clinch first).
- [ ] Repo "New issue" page shows only Clinch contact links.
- [ ] `lsof -nP -i -a -p "$(pgrep -f 'Clinch.app/Contents/MacOS/stable' | paste -sd, -)" | grep ESTABLISHED` → empty with gauges off.
- [ ] Quick visual pass on the still-unverified UI features (usage widget in vertical tabs, repo-name header, Continue/LGTM buttons).

- [ ] **Step 5: Announce** 🎉

---

## Out of Scope (explicitly deferred, tracked for post-launch)

- Deleting unused `warp-logo-*.svg` / onboarding assets — referenced by auth/onboarding code that's compiled (though unreachable) in stable; removing needs a careful asset-macro pass.
- "Update Warp" autoupdate UI strings — the update flow never surfaces in stable (autoupdate polls an unroutable address); rebrand alongside a future in-app updater.
- Renaming the `oz` CLI, internal `dev.warp.*` ids for non-shipped channels, and `warp://cli-agent` OSC sentinel (per CLAUDE.md).
- git2 0.21 / quick-xml 0.41 semver-major bumps (deny.toml documents the accepted advisories).
- Unifying `tools/agent-resume/build-app.sh` onto the `stable` binary.
- Remaining contextual `docs.warp.dev` help links inside settings pages and feature tooltips (`app/src/settings_view/features_page.rs`, `app/src/workspace/view.rs` doc links) — nominative references to upstream docs that still accurately describe the features; sweep them when Clinch has docs of its own.
- Unused `slack-logo.svg` after Task 4 — bundled asset cleanup belongs with the warp-logo asset pass above.

# Clinch ship flow — build a distributable DMG, publish it as a GitHub Release,
# and (optionally) update your own machine. Everything runs locally and free: no
# CI, no GitHub Actions secrets, no macOS runner minutes.
#
#   make candidate               # build + verify every public artifact without publishing
#   make release                 # launch gate → build → verify → GitHub Release
#   make update                  # build, update + relaunch Clinch on THIS machine right away,
#                                # then publish the GitHub Release in the background
#   make release VERSION=v0.2.0  # override the auto date-based tag
#   make release UNIVERSAL=0     # opt into a current-machine-only developer artifact
#
# The default release is self-signed (not notarized). Set REQUIRE_NOTARIZATION=1 when a
# Developer ID/notarized build is available to make artifact verification enforce Gatekeeper.

CLINCH_REPO ?= elliot-ylambda/clinch-terminal

# --- Released app (stable channel, distributed via GitHub Releases) ---
STABLE_APP         ?= Clinch
STABLE_PROFILE_DIR := release-lto
STABLE_BUNDLE      := target/$(STABLE_PROFILE_DIR)/bundle/osx/$(STABLE_APP).app
INSTALLED_APP      := /Applications/$(STABLE_APP).app
RELEASE_DMG        := target/$(STABLE_PROFILE_DIR)/bundle/osx/$(STABLE_APP).dmg
# The clinch.sh site's Install button links to Clinch.app.zip on the latest
# release, so every release must attach the zip alongside the DMG.
RELEASE_ZIP        := target/$(STABLE_PROFILE_DIR)/bundle/osx/$(STABLE_APP).app.zip
RELEASE_SHA        := $(RELEASE_ZIP).sha256
RELEASE_MANIFEST   := target/$(STABLE_PROFILE_DIR)/bundle/osx/$(STABLE_APP).update.json
RELEASE_SIGNATURE  := target/$(STABLE_PROFILE_DIR)/bundle/osx/$(STABLE_APP).update.sig
# Public candidates/releases support both Intel and Apple Silicon by default.
# UNIVERSAL=0 remains available for a faster, non-public developer artifact.
UNIVERSAL          ?= 1
BUNDLE_ARCH_FLAG   := $(if $(filter 1 true yes,$(UNIVERSAL)),,--nouniversal)
# Freeze the default tag at parse time: `release` re-expands $(VERSION) after the
# multi-minute _bundle step, and a recursively-expanded default would re-run `date`
# there, stamping an app version that mismatches the published tag.
ifeq ($(origin VERSION), undefined)
VERSION            := v0.$(shell date +%Y.%m.%d.%H%M)
endif
ifeq ($(origin UPDATE_SEQUENCE), undefined)
UPDATE_SEQUENCE    := $(shell date +%s)
endif

# The private update key stays outside the checkout. Candidate/release packaging fails closed when
# it is missing; only the corresponding public key is committed and bundled.
CLINCH_UPDATE_SIGNING_KEY ?= $(HOME)/.config/clinch/update-signing-key.pem

# create-dmg formats the DMG window (background + icon layout) by scripting Finder via
# AppleScript, which times out (-1712) in headless/automation contexts (agents, CI, no
# interactive Finder). This local ship flow favors a reliable build over DMG cosmetics, so
# default to skipping that step — the DMG is still fully functional. Override with
# `make release SKIP_DMG_APPLESCRIPT=0` for the custom layout when running interactively.
SKIP_DMG_APPLESCRIPT ?= 1
export SKIP_DMG_APPLESCRIPT

# Bypass the latest-main guard (require-latest-main) and build the current HEAD
# as-is — intentional feature-branch or dirty-tree test builds. Exported so the
# flag reaches script/require-latest-main when passed as `make release SKIP_SYNC=1`.
SKIP_SYNC ?=
export SKIP_SYNC

define RELEASE_NOTES
Works on macOS. **Easiest install** - paste this in any terminal. The installer
verifies the published SHA-256 and app signature, configures Claude/Codex session
resume, installs notification plugins when available, and opens Clinch:

    curl -fsSL https://clinch.sh/install | sh

Or download **$(STABLE_APP).dmg** below, open it, and drag $(STABLE_APP) to
Applications. ($(STABLE_APP).app.zip is the same app - it's what the install
script downloads.) Agent resume itself has no jq, Homebrew, clone, or shell-restart
requirement.

Manual downloads get quarantined because this build is self-signed (not
notarized), and macOS 15+ removed the right-click **Open** bypass. Either
clear the flag once:

    xattr -dr com.apple.quarantine /Applications/$(STABLE_APP).app

or try to open the app, then approve it under System Settings > Privacy &
Security > **Open Anyway**.
endef
export RELEASE_NOTES

.DEFAULT_GOAL := help
.PHONY: help candidate release update release-check require-latest-main \
	_require-create-dmg _bundle _package _verify _publish

help: ## List available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
	  | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2}'

# Internal: build + self-sign the bundle (and its DMG). Shared by candidate/release/update.
_bundle: release-check require-latest-main _require-create-dmg
	GIT_RELEASE_TAG="$(VERSION)" CLINCH_UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)" \
	  ./script/bundle -c stable --selfsign $(BUNDLE_ARCH_FLAG)

# Internal: package and verify the exact bytes users will download.
_package:
	ditto -c -k --keepParent "$(STABLE_BUNDLE)" "$(RELEASE_ZIP)"
	cd target/$(STABLE_PROFILE_DIR)/bundle/osx && shasum -a 256 "$(STABLE_APP).app.zip" > "$(STABLE_APP).app.zip.sha256"
	CLINCH_UPDATE_SIGNING_KEY="$(CLINCH_UPDATE_SIGNING_KEY)" \
	  ./script/clinch-update-manifest generate "$(STABLE_BUNDLE)" "$(RELEASE_ZIP)" \
	  "$(VERSION)" "$(UPDATE_SEQUENCE)" "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)"

_verify:
	REQUIRE_NOTARIZATION="$(REQUIRE_NOTARIZATION)" REQUIRE_UNIVERSAL="$(UNIVERSAL)" \
	  ./script/verify-clinch-release \
	  "$(STABLE_BUNDLE)" "$(RELEASE_ZIP)" "$(RELEASE_SHA)" "$(RELEASE_DMG)" "$(VERSION)" \
	  "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)" "$(UPDATE_SEQUENCE)"

# Internal: package, verify, then publish. Callers pin VERSION because its default is
# time-based and must remain identical across every sub-make.
_publish:
	@$(MAKE) _package VERSION="$(VERSION)" UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)"
	@$(MAKE) _verify VERSION="$(VERSION)" UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)" REQUIRE_NOTARIZATION="$(REQUIRE_NOTARIZATION)"
	gh release create "$(VERSION)" "$(RELEASE_DMG)" "$(RELEASE_ZIP)" "$(RELEASE_SHA)" \
	  "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)" \
	  --repo $(CLINCH_REPO) \
	  --title "$(STABLE_APP) $(VERSION)" \
	  --notes "$$RELEASE_NOTES"
	@echo "✓ Published $(VERSION): https://github.com/$(CLINCH_REPO)/releases/tag/$(VERSION)"

candidate: _bundle ## Build and verify universal launch artifacts without publishing (VERSION=v0.x)
	@$(MAKE) _package VERSION="$(VERSION)" UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)"
	@$(MAKE) _verify VERSION="$(VERSION)" UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)" REQUIRE_NOTARIZATION="$(REQUIRE_NOTARIZATION)"
	@echo "✓ Candidate ready: $(RELEASE_DMG), $(RELEASE_ZIP), $(RELEASE_SHA)"

release: _bundle ## Run the launch gate, verify artifacts, and publish a GitHub Release
	@$(MAKE) _publish VERSION="$(VERSION)" UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)"

update: _bundle ## Build, update + relaunch Clinch on THIS machine right away; publish finishes in the background
	@echo "→ Updating $(INSTALLED_APP) — $(STABLE_APP) will quit and relaunch on the new build; publishing $(VERSION) continues in the background…"
	@# One detached shell for BOTH the swap and the publish: when this command is
	@# issued from inside Clinch, quitting Clinch kills make itself, so anything
	@# that must still run has to live in the nohup'd shell. The swap goes first
	@# (seconds — you're on the new build immediately); the zip + upload follow.
	@# `;` not `&&`: publish regardless of the swap outcome, as before.
	@nohup sh -c './script/update-installed-clinch "$(STABLE_APP)" "$(STABLE_BUNDLE)"; $(MAKE) _publish VERSION="$(VERSION)" UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)"' \
	  >"$$HOME/Library/Logs/clinch-self-update.log" 2>&1 &
	@echo "✓ Swap + publish running in the background (log: ~/Library/Logs/clinch-self-update.log)"

# Keep the installed agent-resume capture layer (hooks + ~/.warp/agent-resume-bin) in
# sync with the repo whenever we ship from this machine. Declared as a standalone
# prerequisite line so it survives reworks of the release/update targets. Idempotent,
# local-only, and independent of the app build.
.PHONY: agent-resume
_bundle: agent-resume
agent-resume: ## Install/refresh the agent-resume capture layer (hooks + ~/.warp/agent-resume-bin)
	bash tools/agent-resume/install.sh

release-check: ## Run the complete Clinch source launch gate (format, tests, lint, advisories)
	./script/launch-check

require-latest-main: ## Fast-forward main to clinch/main before building (SKIP_SYNC=1 to bypass)
	./script/require-latest-main

_require-create-dmg:
	@if [ "$(SKIP_DMG_APPLESCRIPT)" = "1" ]; then \
	  command -v hdiutil >/dev/null 2>&1 || { echo "✗ hdiutil is required to build the DMG"; exit 1; }; \
	else \
	  command -v create-dmg >/dev/null 2>&1 || { \
	    echo "✗ create-dmg is required for the styled DMG. Install: brew install create-dmg"; exit 1; }; \
	fi

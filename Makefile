# Clinch ship flow — build a distributable DMG, publish it as a GitHub Release,
# and update your own machine. Everything runs locally and free: no CI, no
# GitHub Actions secrets, no macOS runner minutes.
#
#   make ship                    # update this machine AND publish a release
#   make release                 # build a self-signed Clinch.dmg → GitHub Release
#   make release VERSION=v0.2.0  # override the auto date-based tag
#   make release UNIVERSAL=1     # build a universal (Intel+ARM) DMG (slower)
#   make install-local           # build/install your personal WarpLocal.app
#
# The released app is self-signed (not notarized); the release notes tell users
# how to open it past Gatekeeper.

CLINCH_REPO ?= elliot-ylambda/clinch-terminal

# --- Released app (stable channel, distributed via GitHub Releases) ---
STABLE_APP         ?= Clinch
STABLE_PROFILE_DIR := release-lto
RELEASE_DMG        := target/$(STABLE_PROFILE_DIR)/bundle/osx/$(STABLE_APP).dmg
# Universal (Intel+ARM) is much slower; default to this machine's arch only.
BUNDLE_ARCH_FLAG   := $(if $(UNIVERSAL),,--nouniversal)
VERSION            ?= v0.$(shell date +%Y.%m.%d.%H%M)

# create-dmg formats the DMG window (background + icon layout) by scripting Finder via
# AppleScript, which times out (-1712) in headless/automation contexts (agents, CI, no
# interactive Finder). This local ship flow favors a reliable build over DMG cosmetics, so
# default to skipping that step — the DMG is still fully functional. Override with
# `make ship SKIP_DMG_APPLESCRIPT=0` for the custom layout when running interactively.
SKIP_DMG_APPLESCRIPT ?= 1
export SKIP_DMG_APPLESCRIPT

# --- Personal local dev app (local channel, never auto-updates) ---
LOCAL_APP    := WarpLocal.app
LOCAL_BUNDLE := target/release-lto-debug_assertions/bundle/osx/$(LOCAL_APP)

define RELEASE_NOTES
Download **$(STABLE_APP).dmg** below, open it, and drag $(STABLE_APP) to Applications.

This build is self-signed (not notarized). On first launch, right-click
$(STABLE_APP) → **Open**, or clear the quarantine flag:

    xattr -dr com.apple.quarantine /Applications/$(STABLE_APP).app
endef
export RELEASE_NOTES

.DEFAULT_GOAL := help
.PHONY: help release install-local ship

help: ## List available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
	  | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2}'

release: _require-create-dmg ## Build a self-signed DMG and publish a GitHub Release (VERSION=v0.x, UNIVERSAL=1)
	./script/bundle -c stable --selfsign $(BUNDLE_ARCH_FLAG)
	gh release create "$(VERSION)" "$(RELEASE_DMG)" \
	  --repo $(CLINCH_REPO) \
	  --title "$(STABLE_APP) $(VERSION)" \
	  --notes "$$RELEASE_NOTES"
	@echo "✓ Published $(VERSION): https://github.com/$(CLINCH_REPO)/releases/tag/$(VERSION)"

install-local: _require-create-dmg ## Build the local channel and install /Applications/WarpLocal.app
	./script/bundle -c local --selfsign --nouniversal
	@rm -rf "/Applications/$(LOCAL_APP)"
	cp -R "$(LOCAL_BUNDLE)" "/Applications/$(LOCAL_APP)"
	@echo "✓ Installed /Applications/$(LOCAL_APP)"

ship: install-local release ## Update this machine AND publish a release

_require-create-dmg:
	@command -v create-dmg >/dev/null 2>&1 || { \
	  echo "✗ create-dmg required by script/bundle. Install:  brew install create-dmg"; exit 1; }

# Clinch public-preview build flow.
#
#   make candidate                    Build and verify locally; never publish.
#   make release                      Build, sign, verify the uploaded draft, and publish locally.
#   make update                       Install the latest authenticated public release manually.
#   make dev                          Build and run isolated Clinch Dev.
#   make prune                        Delete regenerable build caches by hand.
#
# The build targets prune stale caches first so a release is not aborted by the
# 40 GiB free-space floor; CLINCH_SKIP_PRUNE=1 disables that.

CLINCH_REPO ?= elliot-ylambda/clinch-terminal

STABLE_APP         ?= Clinch
STABLE_PROFILE_DIR := release-lto
STABLE_BUNDLE      := target/$(STABLE_PROFILE_DIR)/bundle/osx/$(STABLE_APP).app
ARM64_BUNDLE       := target/aarch64-apple-darwin/$(STABLE_PROFILE_DIR)/bundle/osx/$(STABLE_APP).app
X86_64_BUNDLE      := target/x86_64-apple-darwin/$(STABLE_PROFILE_DIR)/bundle/osx/$(STABLE_APP).app
RELEASE_DIR        := target/$(STABLE_PROFILE_DIR)/bundle/osx
RELEASE_DMG        := $(RELEASE_DIR)/$(STABLE_APP).dmg
RELEASE_ZIP        := $(RELEASE_DIR)/$(STABLE_APP).app.zip
RELEASE_SHA        := $(RELEASE_ZIP).sha256
RELEASE_DMG_SHA    := $(RELEASE_DMG).sha256
X86_64_DMG         := $(RELEASE_DIR)/$(STABLE_APP)-x86_64.dmg
X86_64_ZIP         := $(RELEASE_DIR)/$(STABLE_APP)-x86_64.app.zip
X86_64_SHA         := $(X86_64_ZIP).sha256
X86_64_DMG_SHA     := $(X86_64_DMG).sha256
RELEASE_MANIFEST   := $(RELEASE_DIR)/$(STABLE_APP).update.json
RELEASE_SIGNATURE  := $(RELEASE_DIR)/$(STABLE_APP).update.sig
RELEASE_SSHSIG     := $(RELEASE_DIR)/$(STABLE_APP).update.sshsig

# The default release publishes separate Apple Silicon and Intel bundles so each
# Mac downloads only its native executable. UNIVERSAL=1 retains the legacy fat
# bundle as an explicit release option.
UNIVERSAL         ?= 0
UNIVERSAL_ENABLED := $(if $(filter 1 true yes,$(UNIVERSAL)),1,0)
ifeq ($(origin VERSION), undefined)
VERSION := v0.$(shell date +%Y.%m.%d.%H%M)
endif
CLINCH_AUTO_VERSION := $(if $(filter file,$(origin VERSION)),1,0)
UPDATE_SEQUENCE ?= auto

CLINCH_UPDATE_SIGNING_KEY  ?= $(HOME)/.config/clinch/update-signing-key.pem
CLINCH_RELEASE_SIGNING_KEY ?= $(HOME)/.config/clinch/release-signing-key

# How cold a build cache must be before an automatic prune reclaims it. A week
# keeps everything the current branch and the last release still benefit from.
PRUNE_DAYS ?= 7
SKIP_DMG_APPLESCRIPT       ?= 1
export SKIP_DMG_APPLESCRIPT

define RELEASE_NOTES
Clinch $(VERSION) is an unnotarized public preview for macOS 14 or later on Intel and Apple Silicon.

Install with `curl -fsSL https://clinch.sh/install | sh` — the authenticated install.sh asset
verifies this exact release, stages Clinch in Applications, and opens it. Command-line downloads
are not quarantined, so no Gatekeeper approval is needed. To install manually instead, download
$(if $(filter 1,$(UNIVERSAL_ENABLED)),Clinch.dmg,Clinch.dmg on Apple Silicon or Clinch-x86_64.dmg on Intel), authenticate
Clinch.checksums.txt with Clinch.checksums.sshsig, compare the DMG SHA-256, and drag Clinch
to Applications; then use System Settings > Privacy & Security >
Open Anyway when macOS blocks the browser-downloaded first launch.

Session restore is enabled by default, using managed local Claude Code and Codex hooks. You can
turn session capture off or back on from Clinch Settings. This release checks signed update
metadata at most daily and shows **Update Clinch** in the header; installation requires explicit
consent. Builds older than the updater bridge `v0.2026.07.20.1643` need one final authenticated
manual install. An app that is not user-writable also uses the manual installer instead of
requesting administrator access. Agent completion alerts use standard macOS notifications and are
never sent through Messages or AppleScript.
$(if $(filter 1,$(UNIVERSAL_ENABLED)),,Intel installations that predate native-archive selection need one authenticated manual install for this transition.)

Complete Corresponding Source for this exact release, including locked Cargo dependencies, is
attached to the same GitHub release as `Clinch.source.tar.gz`.
endef
export RELEASE_NOTES

.DEFAULT_GOAL := help
.PHONY: help dev dev-app dev-open candidate release update release-check require-latest-main \
	prune _prune _require-create-dmg _bundle _package _verify _verify-existing \
	_validate-release-layout agent-resume-enable configure-release-repository

help: ## List available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
	  | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-29s\033[0m %s\n", $$1, $$2}'

# Reclaim stale build caches before anything that builds, rather than after it.
# Pruning first is what prevents the out-of-space failure: a release aborts
# unless 40 GiB is free, and immediately after a build every cache is warm, so a
# post-build prune would find nothing to take anyway. Set CLINCH_SKIP_PRUNE=1 to
# leave every cache alone.
_prune:
	@if [ "$(CLINCH_SKIP_PRUNE)" = "1" ]; then \
	  echo "Skipping build-cache prune (CLINCH_SKIP_PRUNE=1)"; \
	else \
	  ./script/reclaim-build-space --quiet --days $(PRUNE_DAYS); \
	fi

prune: ## Delete regenerable build caches (PRUNE_DAYS=0 for every cache)
	./script/reclaim-build-space --days $(PRUNE_DAYS)

dev: _prune ## Incrementally build and run isolated Clinch Dev
	./script/clinch-dev run

dev-app: _prune ## Incrementally build target/debug/bundle/osx/ClinchDev.app
	./script/clinch-dev build

dev-open: _prune ## Rebuild and launch Clinch Dev through LaunchServices
	./script/clinch-dev open

release-check: ## Run the complete source gate locally
	./script/launch-check

require-latest-main: ## Require this checkout to be the clean, current Clinch main commit
	./script/require-latest-main

_require-create-dmg:
	@if [ "$(SKIP_DMG_APPLESCRIPT)" = "1" ]; then \
	  command -v hdiutil >/dev/null 2>&1 || { echo "✗ hdiutil is required"; exit 1; }; \
	else \
	  command -v create-dmg >/dev/null 2>&1 || { \
	    echo "✗ create-dmg is required (brew install create-dmg)"; exit 1; }; \
	fi

_validate-release-layout:
	@case "$(UNIVERSAL)" in 0|1|true|false|yes|no) ;; \
	  *) echo "UNIVERSAL must be 0 or 1" >&2; exit 1 ;; esac

_bundle: _validate-release-layout _require-create-dmg
	@test "$(UPDATE_SEQUENCE)" != auto || { echo "UPDATE_SEQUENCE must be resolved first" >&2; exit 1; }
	@if [ "$(UNIVERSAL_ENABLED)" = 1 ]; then \
	  GIT_RELEASE_TAG="$(VERSION)" CLINCH_UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)" \
	    ./script/bundle -c stable --selfsign; \
	else \
	  schema_cache="$(RELEASE_DIR)/settings-schema.split-release.json"; \
	  rm -f "$$schema_cache"; \
	  GIT_RELEASE_TAG="$(VERSION)" CLINCH_UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)" \
	    SETTINGS_SCHEMA_CACHE="$$schema_cache" \
	    ./script/bundle -c stable --selfsign --arch aarch64; \
	  GIT_RELEASE_TAG="$(VERSION)" CLINCH_UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)" \
	    SETTINGS_SCHEMA_CACHE="$$schema_cache" \
	    ./script/bundle -c stable --selfsign --arch x86_64 --dmg-name-suffix x86_64; \
	  rm -rf "$(STABLE_BUNDLE)"; \
	  ditto "$(ARM64_BUNDLE)" "$(STABLE_BUNDLE)"; \
	  rm -f "$$schema_cache"; \
	fi

_package: _bundle
	@test -f "$(CLINCH_UPDATE_SIGNING_KEY)" || { echo "missing update signing key" >&2; exit 1; }
	@test -f "$(CLINCH_RELEASE_SIGNING_KEY)" || { echo "missing release signing key" >&2; exit 1; }
	rm -f "$(RELEASE_ZIP)" "$(RELEASE_SHA)" "$(RELEASE_DMG_SHA)" \
	  "$(X86_64_ZIP)" "$(X86_64_SHA)" "$(X86_64_DMG_SHA)" \
	  "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)" "$(RELEASE_SSHSIG)"
	@if [ "$(UNIVERSAL_ENABLED)" = 1 ]; then \
	  ditto -c -k --keepParent "$(STABLE_BUNDLE)" "$(RELEASE_ZIP)"; \
	else \
	  ditto -c -k --keepParent "$(ARM64_BUNDLE)" "$(RELEASE_ZIP)"; \
	  ditto -c -k --keepParent "$(X86_64_BUNDLE)" "$(X86_64_ZIP)"; \
	fi
	cd "$(RELEASE_DIR)" && shasum -a 256 "$(STABLE_APP).app.zip" > "$(STABLE_APP).app.zip.sha256"
	cd "$(RELEASE_DIR)" && shasum -a 256 "$(STABLE_APP).dmg" > "$(STABLE_APP).dmg.sha256"
	@if [ "$(UNIVERSAL_ENABLED)" != 1 ]; then \
	  cd "$(RELEASE_DIR)" && shasum -a 256 "$(STABLE_APP)-x86_64.app.zip" \
	    > "$(STABLE_APP)-x86_64.app.zip.sha256"; \
	  cd "$(RELEASE_DIR)" && shasum -a 256 "$(STABLE_APP)-x86_64.dmg" \
	    > "$(STABLE_APP)-x86_64.dmg.sha256"; \
	fi
	@if [ "$(UNIVERSAL_ENABLED)" = 1 ]; then \
	  CLINCH_UPDATE_SIGNING_KEY="$(CLINCH_UPDATE_SIGNING_KEY)" RELEASE_NOTES="$$RELEASE_NOTES" \
	    ./script/clinch-update-manifest generate "$(STABLE_BUNDLE)" "$(RELEASE_ZIP)" \
	    "$(VERSION)" "$(UPDATE_SEQUENCE)" "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)"; \
	else \
	  CLINCH_UPDATE_SIGNING_KEY="$(CLINCH_UPDATE_SIGNING_KEY)" RELEASE_NOTES="$$RELEASE_NOTES" \
	    ./script/clinch-update-manifest generate "$(ARM64_BUNDLE)" "$(RELEASE_ZIP)" \
	    "$(X86_64_ZIP)" "$(VERSION)" "$(UPDATE_SEQUENCE)" "$(RELEASE_MANIFEST)" \
	    "$(RELEASE_SIGNATURE)"; \
	fi
	ssh-keygen -Y sign -f "$(CLINCH_RELEASE_SIGNING_KEY)" -n clinch-install - \
	  < "$(RELEASE_MANIFEST)" > "$(RELEASE_SSHSIG)"

_verify-existing:
	@if [ "$(UNIVERSAL_ENABLED)" = 1 ]; then \
	  REQUIRE_NOTARIZATION=0 REQUIRE_UNIVERSAL=1 \
	    ./script/verify-clinch-release \
	    "$(STABLE_BUNDLE)" "$(RELEASE_ZIP)" "$(RELEASE_SHA)" "$(RELEASE_DMG)" "$(VERSION)" \
	    "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)" "$(RELEASE_SSHSIG)" "$(UPDATE_SEQUENCE)"; \
	else \
	  REQUIRE_NOTARIZATION=0 REQUIRE_UNIVERSAL=0 REQUIRE_ARCHITECTURE=arm64 \
	    ./script/verify-clinch-release \
	    "$(ARM64_BUNDLE)" "$(RELEASE_ZIP)" "$(RELEASE_SHA)" "$(RELEASE_DMG)" "$(VERSION)" \
	    "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)" "$(RELEASE_SSHSIG)" "$(UPDATE_SEQUENCE)"; \
	  REQUIRE_NOTARIZATION=0 REQUIRE_UNIVERSAL=0 REQUIRE_ARCHITECTURE=x86_64 \
	    ./script/verify-clinch-release \
	    "$(X86_64_BUNDLE)" "$(X86_64_ZIP)" "$(X86_64_SHA)" "$(X86_64_DMG)" "$(VERSION)" \
	    "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)" "$(RELEASE_SSHSIG)" "$(UPDATE_SEQUENCE)"; \
	fi

_verify: _package
	$(MAKE) _verify-existing VERSION="$(VERSION)" UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)" \
	  UNIVERSAL="$(UNIVERSAL)"

candidate: _prune release-check ## Build and verify native Intel and Apple Silicon candidates
	@sequence="$(UPDATE_SEQUENCE)"; \
	if [ "$$sequence" = auto ]; then sequence="$$(./script/next-clinch-update-sequence)"; fi; \
	$(MAKE) _verify VERSION="$(VERSION)" UPDATE_SEQUENCE="$$sequence" UNIVERSAL="$(UNIVERSAL)"
	@echo "✓ Verified local candidate $(VERSION). No tag or release was created."

release: _prune ## Build, sign, remotely verify, and publish without GitHub Actions
	@UNIVERSAL="$(UNIVERSAL)" \
	  CLINCH_REPO="$(CLINCH_REPO)" \
	  CLINCH_UPDATE_SIGNING_KEY="$(CLINCH_UPDATE_SIGNING_KEY)" \
	  CLINCH_RELEASE_SIGNING_KEY="$(CLINCH_RELEASE_SIGNING_KEY)" \
	  VERSION="$(VERSION)" \
	  CLINCH_AUTO_VERSION="$(CLINCH_AUTO_VERSION)" \
	  ./script/release-from-clean-worktree

update: ## Install the latest authenticated public release (Clinch must be quit)
	./install.sh
	@$(MAKE) --no-print-directory _prune

agent-resume-enable: ## Enable or repair local Claude/Codex session capture
	bash tools/agent-resume/install.sh enable

configure-release-repository: ## Apply GitHub branch, scanning, and local-release policy
	./script/configure-clinch-release-repository

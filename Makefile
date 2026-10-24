# Clinch public-preview build flow.
#
#   make candidate                    Build and verify locally; never publish.
#   make release                      Build, sign, verify the uploaded draft, and publish locally.
#   make update                       Install the latest authenticated public release manually.
#   make dev                          Build and run isolated Clinch Dev.
#   make prune                        Delete regenerable build caches by hand.
#   make worktrees                    Preview finished worktree cleanup.
#   make resources                    Watch this Clinch instance's CPU and memory.
#
# The build targets prune stale caches first so a release is not aborted by the
# 40 GiB free-space floor; CLINCH_SKIP_PRUNE=1 disables that.

CLINCH_REPO ?= elliot-ylambda/clinch-terminal

# Builds and the release gate run at a lower CPU priority so the Mac stays responsive meanwhile.
RELEASE_NICE ?= 10

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

# Releases ship Apple Silicon only. Build Intel only when it is explicitly wanted:
# INTEL=1 adds a separate native Intel bundle, and UNIVERSAL=1 ships one legacy fat
# bundle that also runs on Intel.
UNIVERSAL         ?= 0
INTEL             ?= 0
UNIVERSAL_ENABLED := $(if $(filter 1 true yes,$(UNIVERSAL)),1,0)
INTEL_ENABLED     := $(if $(filter 1 true yes,$(INTEL)),1,0)
RELEASE_LAYOUT    := $(if $(filter 1,$(UNIVERSAL_ENABLED)),universal,$(if $(filter 1,$(INTEL_ENABLED)),native,arm64))
# Intel Macs are pointed at this release once a release ships without Intel.
LAST_INTEL_RELEASE = $(shell sed -n 's/^LAST_INTEL_RELEASE="\(.*\)"$$/\1/p' install.sh 2>/dev/null)
ifeq ($(origin VERSION), undefined)
VERSION := v0.$(shell date +%Y.%m.%d.%H%M)
endif
CLINCH_AUTO_VERSION := $(if $(filter file,$(origin VERSION)),1,0)
UPDATE_SEQUENCE ?= auto

CLINCH_UPDATE_SIGNING_KEY  ?= $(HOME)/.config/clinch/update-signing-key.pem
CLINCH_RELEASE_SIGNING_KEY ?= $(HOME)/.config/clinch/release-signing-key

# Every release is signed by one committed self-signed certificate. macOS keys privacy grants
# (Screen Recording, Accessibility) on the code-signing requirement; an ad-hoc signature pins that
# requirement to one build's hash, so each update silently revoked every grant and re-prompted.
# The private key lives in a dedicated keychain that script/bundle unlocks only while signing.
CLINCH_CODESIGN_CERT                   := resources/release/clinch-codesign-cert.pem
CLINCH_CODESIGN_IDENTITY                = $(shell /usr/bin/openssl x509 -in $(CLINCH_CODESIGN_CERT) -noout -fingerprint -sha1 | cut -d= -f2 | tr -d :)
CLINCH_CODESIGN_KEYCHAIN              ?= $(HOME)/.config/clinch/codesign.keychain-db
CLINCH_CODESIGN_KEYCHAIN_PASSWORD_FILE ?= $(HOME)/.config/clinch/codesign-keychain-password

# How cold a build cache must be before an automatic prune reclaims it. A week
# keeps everything the current branch and the last release still benefit from.
PRUNE_DAYS ?= 7
SKIP_DMG_APPLESCRIPT       ?= 1
export SKIP_DMG_APPLESCRIPT

define RELEASE_NOTES
Clinch $(VERSION) is an unnotarized public preview for macOS 14 or later on $(if $(filter arm64,$(RELEASE_LAYOUT)),Apple Silicon,Intel and Apple Silicon).

Install with `curl -fsSL https://clinch.sh/install | sh` — the authenticated install.sh asset
verifies this exact release, stages Clinch in Applications, and opens it. Command-line downloads
are not quarantined, so no Gatekeeper approval is needed. To install manually instead, download
$(if $(filter native,$(RELEASE_LAYOUT)),Clinch.dmg on Apple Silicon or Clinch-x86_64.dmg on Intel,Clinch.dmg), authenticate
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

Clinch releases use one stable signing certificate so macOS privacy permissions can survive updates.
Upgrading from an older ad-hoc or Apple Development signed build may require a one-time permission
repair. If computer use is denied even though Clinch is enabled, quit Clinch, remove its entry with
the minus button from both Accessibility and Screen & System Audio Recording in System Settings >
Privacy & Security, then add the current /Applications/Clinch.app with the plus button and enable
both permissions. Reopen Clinch, resume your agent, and retry. Toggling the existing entries or
restarting alone may leave the old signing identity in place.

New installations only need these permissions when using computer use; ordinary terminal use does
not require them. Setup and recovery instructions are available in Clinch Settings > Computer use.
$(if $(filter native,$(RELEASE_LAYOUT)),Intel installations that predate native-archive selection need one authenticated manual install for this transition.)
$(if $(filter arm64,$(RELEASE_LAYOUT)),This release supports Apple Silicon Macs only. Intel Macs should stay on $(LAST_INTEL_RELEASE) (the last Intel release); install it with `curl -fsSL https://clinch.sh/install | sh -s -- --version $(LAST_INTEL_RELEASE)`.)

Complete Corresponding Source for this exact release, including locked Cargo dependencies, is
attached to the same GitHub release as `Clinch.source.tar.gz`.
endef
export RELEASE_NOTES

.DEFAULT_GOAL := help
.PHONY: help dev dev-app dev-open candidate release update release-check require-latest-main \
	prune worktrees resources _prune _require-create-dmg _bundle _package _verify _verify-existing \
	_check-codesign-identity _validate-release-layout _write-release-checksums agent-resume-enable \
	configure-release-repository

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

worktrees: ## Preview finished worktrees and their disk usage; deletes nothing
	./script/clean-worktrees --size

resources: ## Watch CPU and memory for the Clinch instance bound to this terminal
	./script/watch-clinch-resources

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
	@case "$(INTEL)" in 0|1|true|false|yes|no) ;; \
	  *) echo "INTEL must be 0 or 1" >&2; exit 1 ;; esac
	@test "$(UNIVERSAL_ENABLED)$(INTEL_ENABLED)" != 11 \
	  || { echo "set INTEL=1 or UNIVERSAL=1, not both" >&2; exit 1; }

_check-codesign-identity:
	@test -f "$(CLINCH_CODESIGN_CERT)" && test -n "$(CLINCH_CODESIGN_IDENTITY)" \
	  || { echo "missing or unreadable $(CLINCH_CODESIGN_CERT)" >&2; exit 1; }
	@test -f "$(CLINCH_CODESIGN_KEYCHAIN)" \
	  || { echo "missing code-signing keychain $(CLINCH_CODESIGN_KEYCHAIN)" >&2; exit 1; }
	@./script/unlock-clinch-codesign-keychain \
	  "$(CLINCH_CODESIGN_KEYCHAIN)" "$(CLINCH_CODESIGN_KEYCHAIN_PASSWORD_FILE)" \
	  && security lock-keychain "$(CLINCH_CODESIGN_KEYCHAIN)" \
	  || { echo "$(CLINCH_CODESIGN_KEYCHAIN_PASSWORD_FILE) does not unlock the code-signing keychain" >&2; exit 1; }
	@security find-identity -v -p codesigning "$(CLINCH_CODESIGN_KEYCHAIN)" \
	  | grep -q "$(CLINCH_CODESIGN_IDENTITY)" \
	  || { echo "Clinch Code Signing is not trusted for code signing; see SECURITY.md" >&2; exit 1; }

_bundle: _validate-release-layout _require-create-dmg _check-codesign-identity
	@test "$(UPDATE_SEQUENCE)" != auto || { echo "UPDATE_SEQUENCE must be resolved first" >&2; exit 1; }
	@set -e; \
	export CLINCH_CODESIGN_IDENTITY="$(CLINCH_CODESIGN_IDENTITY)"; \
	export CLINCH_CODESIGN_KEYCHAIN="$(CLINCH_CODESIGN_KEYCHAIN)"; \
	export CLINCH_CODESIGN_KEYCHAIN_PASSWORD_FILE="$(CLINCH_CODESIGN_KEYCHAIN_PASSWORD_FILE)"; \
	if [ "$(RELEASE_LAYOUT)" = universal ]; then \
	  GIT_RELEASE_TAG="$(VERSION)" CLINCH_UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)" \
	    ./script/bundle -c stable --selfsign; \
	elif [ "$(RELEASE_LAYOUT)" = arm64 ]; then \
	  GIT_RELEASE_TAG="$(VERSION)" CLINCH_UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)" \
	    ./script/bundle -c stable --selfsign --arch aarch64; \
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

_write-release-checksums:
	cd "$(RELEASE_DIR)" && shasum -a 256 "$(STABLE_APP).app.zip" > "$(STABLE_APP).app.zip.sha256"
	cd "$(RELEASE_DIR)" && shasum -a 256 "$(STABLE_APP).dmg" > "$(STABLE_APP).dmg.sha256"
	@if [ "$(RELEASE_LAYOUT)" = native ]; then \
	  cd "$(RELEASE_DIR)" && \
	    shasum -a 256 "$(STABLE_APP)-x86_64.app.zip" \
	      > "$(STABLE_APP)-x86_64.app.zip.sha256" && \
	    shasum -a 256 "$(STABLE_APP)-x86_64.dmg" \
	      > "$(STABLE_APP)-x86_64.dmg.sha256"; \
	fi

_package: _bundle
	@test -f "$(CLINCH_UPDATE_SIGNING_KEY)" || { echo "missing update signing key" >&2; exit 1; }
	@test -f "$(CLINCH_RELEASE_SIGNING_KEY)" || { echo "missing release signing key" >&2; exit 1; }
	rm -f "$(RELEASE_ZIP)" "$(RELEASE_SHA)" "$(RELEASE_DMG_SHA)" \
	  "$(X86_64_ZIP)" "$(X86_64_SHA)" "$(X86_64_DMG)" "$(X86_64_DMG_SHA)" \
	  "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)" "$(RELEASE_SSHSIG)"
	@if [ "$(RELEASE_LAYOUT)" = universal ]; then \
	  ditto -c -k --keepParent "$(STABLE_BUNDLE)" "$(RELEASE_ZIP)"; \
	else \
	  ditto -c -k --keepParent "$(ARM64_BUNDLE)" "$(RELEASE_ZIP)"; \
	fi
	@if [ "$(RELEASE_LAYOUT)" = native ]; then \
	  ditto -c -k --keepParent "$(X86_64_BUNDLE)" "$(X86_64_ZIP)"; \
	fi
	$(MAKE) --no-print-directory _write-release-checksums UNIVERSAL="$(UNIVERSAL)" INTEL="$(INTEL)"
	@if [ "$(RELEASE_LAYOUT)" = universal ]; then \
	  CLINCH_UPDATE_SIGNING_KEY="$(CLINCH_UPDATE_SIGNING_KEY)" RELEASE_NOTES="$$RELEASE_NOTES" \
	    ./script/clinch-update-manifest generate "$(STABLE_BUNDLE)" "$(RELEASE_ZIP)" \
	    "$(VERSION)" "$(UPDATE_SEQUENCE)" "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)"; \
	elif [ "$(RELEASE_LAYOUT)" = arm64 ]; then \
	  CLINCH_UPDATE_SIGNING_KEY="$(CLINCH_UPDATE_SIGNING_KEY)" RELEASE_NOTES="$$RELEASE_NOTES" \
	  CLINCH_MANIFEST_ARCHITECTURES=arm64 \
	    ./script/clinch-update-manifest generate "$(ARM64_BUNDLE)" "$(RELEASE_ZIP)" \
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
	@if [ "$(RELEASE_LAYOUT)" = universal ]; then \
	  REQUIRE_NOTARIZATION=0 RELEASE_LAYOUT=universal \
	    ./script/verify-clinch-release \
	    "$(STABLE_BUNDLE)" "$(RELEASE_ZIP)" "$(RELEASE_SHA)" "$(RELEASE_DMG)" "$(VERSION)" \
	    "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)" "$(RELEASE_SSHSIG)" "$(UPDATE_SEQUENCE)"; \
	else \
	  REQUIRE_NOTARIZATION=0 RELEASE_LAYOUT="$(RELEASE_LAYOUT)" REQUIRE_ARCHITECTURE=arm64 \
	    ./script/verify-clinch-release \
	    "$(ARM64_BUNDLE)" "$(RELEASE_ZIP)" "$(RELEASE_SHA)" "$(RELEASE_DMG)" "$(VERSION)" \
	    "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)" "$(RELEASE_SSHSIG)" "$(UPDATE_SEQUENCE)"; \
	fi
	@if [ "$(RELEASE_LAYOUT)" = native ]; then \
	  REQUIRE_NOTARIZATION=0 RELEASE_LAYOUT=native REQUIRE_ARCHITECTURE=x86_64 \
	    ./script/verify-clinch-release \
	    "$(X86_64_BUNDLE)" "$(X86_64_ZIP)" "$(X86_64_SHA)" "$(X86_64_DMG)" "$(VERSION)" \
	    "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)" "$(RELEASE_SSHSIG)" "$(UPDATE_SEQUENCE)"; \
	fi

_verify: _package
	$(MAKE) _verify-existing VERSION="$(VERSION)" UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)" \
	  UNIVERSAL="$(UNIVERSAL)" INTEL="$(INTEL)"

candidate: _prune ## Build and verify a local candidate (Apple Silicon; INTEL=1 adds Intel)
	nice -n $(RELEASE_NICE) $(MAKE) --no-print-directory release-check
	@sequence="$(UPDATE_SEQUENCE)"; \
	if [ "$$sequence" = auto ]; then sequence="$$(./script/next-clinch-update-sequence)"; fi; \
	nice -n $(RELEASE_NICE) $(MAKE) _verify VERSION="$(VERSION)" UPDATE_SEQUENCE="$$sequence" \
	  UNIVERSAL="$(UNIVERSAL)" INTEL="$(INTEL)"
	@echo "✓ Verified local candidate $(VERSION). No tag or release was created."

release: _prune ## Build, sign, remotely verify, and publish without GitHub Actions
	@UNIVERSAL="$(UNIVERSAL)" \
	  INTEL="$(INTEL)" \
	  CLINCH_REPO="$(CLINCH_REPO)" \
	  CLINCH_UPDATE_SIGNING_KEY="$(CLINCH_UPDATE_SIGNING_KEY)" \
	  CLINCH_RELEASE_SIGNING_KEY="$(CLINCH_RELEASE_SIGNING_KEY)" \
	  VERSION="$(VERSION)" \
	  CLINCH_AUTO_VERSION="$(CLINCH_AUTO_VERSION)" \
	  nice -n $(RELEASE_NICE) ./script/release-from-clean-worktree

update: ## Install the latest authenticated public release (Clinch must be quit)
	./install.sh
	@$(MAKE) --no-print-directory _prune

agent-resume-enable: ## Enable or repair local Claude/Codex session capture
	bash tools/agent-resume/install.sh enable

configure-release-repository: ## Apply GitHub branch, scanning, and local-release policy
	./script/configure-clinch-release-repository

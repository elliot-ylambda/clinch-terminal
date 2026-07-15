# Clinch public-preview build flow.
#
#   make candidate                    Build and verify locally; never publish.
#   make release                      Build, sign, verify the uploaded draft, and publish locally.
#   make update                       Install the latest authenticated public release manually.
#   make dev                          Build and run isolated Clinch Dev.

CLINCH_REPO ?= elliot-ylambda/clinch-terminal

STABLE_APP         ?= Clinch
STABLE_PROFILE_DIR := release-lto
STABLE_BUNDLE      := target/$(STABLE_PROFILE_DIR)/bundle/osx/$(STABLE_APP).app
RELEASE_DIR        := target/$(STABLE_PROFILE_DIR)/bundle/osx
RELEASE_DMG        := $(RELEASE_DIR)/$(STABLE_APP).dmg
RELEASE_ZIP        := $(RELEASE_DIR)/$(STABLE_APP).app.zip
RELEASE_SHA        := $(RELEASE_ZIP).sha256
RELEASE_DMG_SHA    := $(RELEASE_DMG).sha256
RELEASE_MANIFEST   := $(RELEASE_DIR)/$(STABLE_APP).update.json
RELEASE_SIGNATURE  := $(RELEASE_DIR)/$(STABLE_APP).update.sig
RELEASE_SSHSIG     := $(RELEASE_DIR)/$(STABLE_APP).update.sshsig

UNIVERSAL        ?= 1
BUNDLE_ARCH_FLAG := $(if $(filter 1 true yes,$(UNIVERSAL)),,--nouniversal)
ifeq ($(origin VERSION), undefined)
VERSION := v0.$(shell date +%Y.%m.%d.%H%M)
endif
CLINCH_AUTO_VERSION := $(if $(filter file,$(origin VERSION)),1,0)
UPDATE_SEQUENCE ?= auto
QA_RECORD ?= auto
QA_TESTED_MACOS_VERSIONS ?= auto
QA_CONFIRMED ?= false
QA_FIRST_INSTALL ?= false
QA_AUTHENTICATED_UPGRADE ?= false
QA_SESSION_INTEGRATION ?= false
QA_UNINSTALL ?= false
QA_OFFLINE_STARTUP ?= false
QA_APPLE_SILICON_SMOKE ?= false
QA_INTEL_SMOKE ?= false

CLINCH_UPDATE_SIGNING_KEY  ?= $(HOME)/.config/clinch/update-signing-key.pem
CLINCH_RELEASE_SIGNING_KEY ?= $(HOME)/.config/clinch/release-signing-key
SKIP_DMG_APPLESCRIPT       ?= 1
export SKIP_DMG_APPLESCRIPT

define RELEASE_NOTES
Clinch $(VERSION) is an unnotarized public preview for macOS 14 or later on Intel and Apple Silicon.

Download Clinch.dmg, authenticate Clinch.checksums.txt with Clinch.checksums.sshsig, compare the
DMG SHA-256, and drag Clinch to Applications. Then use System Settings > Privacy & Security >
Open Anyway if macOS blocks the first launch. The authenticated install.sh asset is available as
a secondary convenience.

Session capture and provider plugins are off by default. Enable session capture from Clinch
Settings only if you want Clinch to add managed Claude Code and Codex hooks. Automatic updates are
disabled for this preview; install a newer authenticated release manually. Two-way iMessage is
optional and local; setup requires Messages Automation and Full Disk Access on the Mac.
endef
export RELEASE_NOTES

.DEFAULT_GOAL := help
.PHONY: help dev dev-app dev-open candidate release update release-check require-latest-main \
	_require-create-dmg _bundle _package _verify agent-resume-enable configure-release-repository

help: ## List available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
	  | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-29s\033[0m %s\n", $$1, $$2}'

dev: ## Incrementally build and run isolated Clinch Dev
	./script/clinch-dev run

dev-app: ## Incrementally build target/debug/bundle/osx/ClinchDev.app
	./script/clinch-dev build

dev-open: ## Rebuild and launch Clinch Dev through LaunchServices
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

_bundle: _require-create-dmg
	@test "$(UPDATE_SEQUENCE)" != auto || { echo "UPDATE_SEQUENCE must be resolved first" >&2; exit 1; }
	GIT_RELEASE_TAG="$(VERSION)" CLINCH_UPDATE_SEQUENCE="$(UPDATE_SEQUENCE)" \
	  ./script/bundle -c stable --selfsign $(BUNDLE_ARCH_FLAG)

_package: _bundle
	@test -f "$(CLINCH_UPDATE_SIGNING_KEY)" || { echo "missing update signing key" >&2; exit 1; }
	@test -f "$(CLINCH_RELEASE_SIGNING_KEY)" || { echo "missing release signing key" >&2; exit 1; }
	rm -f "$(RELEASE_ZIP)" "$(RELEASE_SHA)" "$(RELEASE_DMG_SHA)" \
	  "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)" "$(RELEASE_SSHSIG)"
	ditto -c -k --keepParent "$(STABLE_BUNDLE)" "$(RELEASE_ZIP)"
	cd "$(RELEASE_DIR)" && shasum -a 256 "$(STABLE_APP).app.zip" > "$(STABLE_APP).app.zip.sha256"
	cd "$(RELEASE_DIR)" && shasum -a 256 "$(STABLE_APP).dmg" > "$(STABLE_APP).dmg.sha256"
	CLINCH_UPDATE_SIGNING_KEY="$(CLINCH_UPDATE_SIGNING_KEY)" RELEASE_NOTES="$$RELEASE_NOTES" \
	  ./script/clinch-update-manifest generate "$(STABLE_BUNDLE)" "$(RELEASE_ZIP)" \
	  "$(VERSION)" "$(UPDATE_SEQUENCE)" "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)"
	ssh-keygen -Y sign -f "$(CLINCH_RELEASE_SIGNING_KEY)" -n clinch-install - \
	  < "$(RELEASE_MANIFEST)" > "$(RELEASE_SSHSIG)"

_verify: _package
	REQUIRE_NOTARIZATION=0 REQUIRE_UNIVERSAL="$(UNIVERSAL)" \
	  ./script/verify-clinch-release \
	  "$(STABLE_BUNDLE)" "$(RELEASE_ZIP)" "$(RELEASE_SHA)" "$(RELEASE_DMG)" "$(VERSION)" \
	  "$(RELEASE_MANIFEST)" "$(RELEASE_SIGNATURE)" "$(RELEASE_SSHSIG)" "$(UPDATE_SEQUENCE)"

candidate: release-check ## Build and verify a universal candidate without publishing
	@sequence="$(UPDATE_SEQUENCE)"; \
	if [ "$$sequence" = auto ]; then sequence="$$(./script/next-clinch-update-sequence)"; fi; \
	$(MAKE) _verify VERSION="$(VERSION)" UPDATE_SEQUENCE="$$sequence" UNIVERSAL="$(UNIVERSAL)"
	@echo "✓ Verified local candidate $(VERSION). No tag or release was created."

release: ## Build, sign, remotely verify, and publish without GitHub Actions
	@CLINCH_REPO="$(CLINCH_REPO)" \
	  CLINCH_UPDATE_SIGNING_KEY="$(CLINCH_UPDATE_SIGNING_KEY)" \
	  CLINCH_RELEASE_SIGNING_KEY="$(CLINCH_RELEASE_SIGNING_KEY)" \
	  VERSION="$(VERSION)" \
	  CLINCH_AUTO_VERSION="$(CLINCH_AUTO_VERSION)" \
	  QA_RECORD="$(QA_RECORD)" \
	  QA_TESTED_MACOS_VERSIONS="$(QA_TESTED_MACOS_VERSIONS)" \
	  QA_CONFIRMED="$(QA_CONFIRMED)" \
	  QA_FIRST_INSTALL="$(QA_FIRST_INSTALL)" \
	  QA_AUTHENTICATED_UPGRADE="$(QA_AUTHENTICATED_UPGRADE)" \
	  QA_SESSION_INTEGRATION="$(QA_SESSION_INTEGRATION)" \
	  QA_UNINSTALL="$(QA_UNINSTALL)" \
	  QA_OFFLINE_STARTUP="$(QA_OFFLINE_STARTUP)" \
	  QA_APPLE_SILICON_SMOKE="$(QA_APPLE_SILICON_SMOKE)" \
	  QA_INTEL_SMOKE="$(QA_INTEL_SMOKE)" \
	  ./script/dispatch-clinch-release

update: ## Install the latest authenticated public release (Clinch must be quit)
	./install.sh

agent-resume-enable: ## Explicitly enable local Claude/Codex session capture
	bash tools/agent-resume/install.sh enable

configure-release-repository: ## Apply GitHub branch, scanning, and local-release policy
	./script/configure-clinch-release-repository

BUNDLE_NAME = ClinchDockTilePlugin.docktileplugin
# Where the built bundle lands. The app's build script points this at its own
# OUT_DIR so concurrently built architectures never share an output tree; it
# defaults to the source directory for a bare `make` run.
BUNDLE_DIR ?= .
BUNDLE_ROOT = $(BUNDLE_DIR)/$(BUNDLE_NAME)
PLUGIN_BINARY = $(BUNDLE_ROOT)/Contents/MacOS/ClinchDockTilePlugin
OBJC_FILES = ClinchDockTilePlugin.m
FRAMEWORKS = -framework Cocoa -framework AppKit -framework Foundation
MACOSX_DEPLOYMENT_TARGET ?= 10.14
# Compile a universal binary for both arm64 and x86_64.
CFLAGS = -fobjc-arc -bundle -mmacosx-version-min=$(MACOSX_DEPLOYMENT_TARGET) -arch arm64 -arch x86_64
LDFLAGS = -ObjC

.PHONY: all clean

all: $(PLUGIN_BINARY)

$(PLUGIN_BINARY): $(OBJC_FILES) ClinchDockTilePlugin.h Info.plist $(wildcard Resources/*) ../channels/stable/icon/no-padding/512x512.png
	mkdir -p $(BUNDLE_ROOT)/Contents/MacOS
	clang $(CFLAGS) $(LDFLAGS) $(FRAMEWORKS) $(OBJC_FILES) -o $(PLUGIN_BINARY)
	cp Info.plist $(BUNDLE_ROOT)/Contents/
	mkdir -p $(BUNDLE_ROOT)/Contents/Resources
	cp Resources/* $(BUNDLE_ROOT)/Contents/Resources/
	cp ../channels/stable/icon/no-padding/512x512.png $(BUNDLE_ROOT)/Contents/Resources/clinch.png

clean:
	rm -rf $(BUNDLE_ROOT)

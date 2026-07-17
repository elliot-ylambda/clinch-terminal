BUNDLE_NAME = ClinchDockTilePlugin.docktileplugin
PLUGIN_BINARY = $(BUNDLE_NAME)/Contents/MacOS/ClinchDockTilePlugin
OBJC_FILES = ClinchDockTilePlugin.m
FRAMEWORKS = -framework Cocoa -framework AppKit -framework Foundation
MACOSX_DEPLOYMENT_TARGET ?= 10.14
# Compile a universal binary for both arm64 and x86_64.
CFLAGS = -fobjc-arc -bundle -mmacosx-version-min=$(MACOSX_DEPLOYMENT_TARGET) -arch arm64 -arch x86_64
LDFLAGS = -ObjC

.PHONY: all clean

all: $(PLUGIN_BINARY)

$(PLUGIN_BINARY): $(OBJC_FILES) ClinchDockTilePlugin.h Info.plist $(wildcard Resources/*) ../channels/stable/icon/no-padding/512x512.png
	mkdir -p $(BUNDLE_NAME)/Contents/MacOS
	clang $(CFLAGS) $(LDFLAGS) $(FRAMEWORKS) $(OBJC_FILES) -o $(BUNDLE_NAME)/Contents/MacOS/ClinchDockTilePlugin
	cp Info.plist $(BUNDLE_NAME)/Contents/
	mkdir -p $(BUNDLE_NAME)/Contents/Resources
	cp Resources/* $(BUNDLE_NAME)/Contents/Resources/
	cp ../channels/stable/icon/no-padding/512x512.png $(BUNDLE_NAME)/Contents/Resources/clinch.png

clean:
	rm -rf $(BUNDLE_NAME)

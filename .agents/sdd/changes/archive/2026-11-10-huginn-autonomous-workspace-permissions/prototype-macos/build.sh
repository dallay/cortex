#!/bin/sh
set -eu
cd "$(dirname "$0")"

SDK=$(xcrun --sdk macosx --show-sdk-path)
ROOT="$PWD/build"
APP="$ROOT/AppSandboxPrototype.app"
HELPER="$APP/Contents/Helpers/ProbeHelper"
rm -rf "$ROOT"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Helpers"

xcrun --sdk macosx swiftc -sdk "$SDK" Helper.swift -o "$HELPER"
cat WorkspaceSelection.swift Host.swift > "$ROOT/HostCombined.swift"
xcrun --sdk macosx swiftc -sdk "$SDK" "$ROOT/HostCombined.swift" -framework AppKit -o "$APP/Contents/MacOS/AppSandboxPrototype"
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>AppSandboxPrototype</string>
<key>CFBundleIdentifier</key><string>local.cortex.huginn.appsandboxprototype</string>
<key>CFBundleName</key><string>AppSandboxPrototype</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSPrincipalClass</key><string>NSApplication</string>
</dict></plist>
PLIST

# Ad-hoc signatures are a local feasibility attempt only; no identity or certificate is requested.
codesign --force --sign - --entitlements Helper.entitlements.plist "$HELPER"
codesign --force --sign - --entitlements Host.entitlements.plist "$APP"
codesign --verify --strict --verbose=2 "$HELPER"
codesign --verify --strict --verbose=2 "$APP"
printf 'Built ad-hoc local prototype: %s\n' "$APP"

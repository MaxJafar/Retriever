#!/bin/sh
set -eu
cd "$(dirname "$0")"
OUT="${RETRIEVER_BUILD_OUT:-build/Retriever Candidate.app}"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' ../../retriever-rs/Cargo.toml | head -1 | tr -d '\r')
[ -n "$VERSION" ] || { echo 'Missing engine version' >&2; exit 1; }
BUILD_NUMBER=$(printf '%s' "$VERSION" | sed -n 's/.*alpha\.\([0-9][0-9]*\).*/\1/p')
[ -n "$BUILD_NUMBER" ] || BUILD_NUMBER=1
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources/engine"
SWIFTC=$(xcrun --find swiftc)
SDK=$(xcrun --sdk macosx --show-sdk-path)
# Replacing a previously executed Mach-O in place can retain a cached code
# signature for its inode. Publish fresh files before signing the app bundle.
NATIVE_TEMP="$OUT/Contents/MacOS/.Retriever-new-$$"
ENGINE_TEMP="$OUT/Contents/Resources/engine/.retriever-new-$$"
trap 'rm -f "$NATIVE_TEMP" "$ENGINE_TEMP"' EXIT HUP INT TERM
"$SWIFTC" -sdk "$SDK" -parse-as-library -O -target arm64-apple-macosx14.0 Retriever.swift Updates.swift ExplorerView.swift FolderTree.swift ExplorerOnboarding.swift FolderIntegration.swift QuickOverlay.swift DesktopIntegration.swift ExplorerTabs.swift Transfers.swift SmartProperties.swift WorkspacePopovers.swift ExplorerMilestone.swift RemoteDevices.swift DiskBurner.swift DashboardVisuals.swift DiskSpace.swift DriveHealth.swift -o "$NATIVE_TEMP" -framework SwiftUI -framework AppKit
mv -f "$NATIVE_TEMP" "$OUT/Contents/MacOS/Retriever"
cp Retriever.png "$OUT/Contents/Resources/Retriever.png"
cp ../../release/repository.json "$OUT/Contents/Resources/repository.json"
UPDATER="${RETRIEVER_UPDATER:-../../retriever-rs/target/release/retriever-updater}"
if [ -f "$UPDATER" ]; then cp "$UPDATER" "$OUT/Contents/Resources/engine/retriever-updater"; fi
mkdir -p build/Retriever.iconset
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" Retriever.png --out "build/Retriever.iconset/icon_${size}x${size}.png" >/dev/null
  retina=$((size * 2))
  sips -z "$retina" "$retina" Retriever.png --out "build/Retriever.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns build/Retriever.iconset -o "$OUT/Contents/Resources/Retriever.icns"
cat > "$OUT/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>app.retriever.candidate</string><key>CFBundleName</key><string>Retriever Candidate</string><key>CFBundleExecutable</key><string>Retriever</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleShortVersionString</key><string>0.13.0</string><key>CFBundleVersion</key><string>16</string><key>LSMinimumSystemVersion</key><string>14.0</string><key>CFBundleDocumentTypes</key><array><dict><key>CFBundleTypeName</key><string>Folders</string><key>CFBundleTypeRole</key><string>Viewer</string><key>LSHandlerRank</key><string>Alternate</string><key>LSItemContentTypes</key><array><string>public.folder</string></array></dict></array><key>NSServices</key><array><dict><key>NSMenuItem</key><dict><key>default</key><string>Open in Retriever</string></dict><key>NSMessage</key><string>openFolders</string><key>NSPortName</key><string>Retriever Candidate</string><key>NSSendFileTypes</key><array><string>public.folder</string></array><key>NSRequiredContext</key><dict/><key>NSServiceDescription</key><string>Browse the selected folders in Retriever.</string></dict></array><key>NSHighResolutionCapable</key><true/></dict></plist>
PLIST
if [ -n "${RETRIEVER_ENGINE:-}" ]; then
  cp "$RETRIEVER_ENGINE" "$ENGINE_TEMP"
  mv -f "$ENGINE_TEMP" "$OUT/Contents/Resources/engine/retriever"
fi
/usr/libexec/PlistBuddy -c 'Add :CFBundleIconFile string Retriever.icns' "$OUT/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleName Retriever' "$OUT/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Add :CFBundleDisplayName string Retriever' "$OUT/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Set :NSServices:0:NSPortName Retriever' "$OUT/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $BUILD_NUMBER" "$OUT/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Add :RetrieverEngineVersion string $VERSION" "$OUT/Contents/Info.plist"
codesign --force --deep --sign - "$OUT"

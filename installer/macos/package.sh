#!/bin/bash
# Builds the macOS release files into dist/. Needs VERSION and NAME (macos-arm64 or macos-x64).
set -euo pipefail

base="bolt-explorer-v$VERSION-$NAME"
mkdir -p dist
cp target/release/bolt-explorer "dist/$base"
tar czf "dist/$base.tar.gz" -C target/release bolt-explorer

work=$(mktemp -d)
app="$work/root/Bolt Explorer.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/bolt-explorer "$app/Contents/MacOS/"
sed "s/__VERSION__/$VERSION/g" installer/macos/Info.plist > "$app/Contents/Info.plist"

set_dir="$work/icon.iconset"
mkdir "$set_dir"
for s in 16 32 64 128 256; do sips -z "$s" "$s" assets/logo.png --out "$set_dir/$s.png" >/dev/null; done
cp "$set_dir/16.png" "$set_dir/icon_16x16.png"
cp "$set_dir/32.png" "$set_dir/icon_16x16@2x.png"
cp "$set_dir/32.png" "$set_dir/icon_32x32.png"
cp "$set_dir/64.png" "$set_dir/icon_32x32@2x.png"
cp "$set_dir/128.png" "$set_dir/icon_128x128.png"
cp "$set_dir/256.png" "$set_dir/icon_128x128@2x.png"
cp "$set_dir/256.png" "$set_dir/icon_256x256.png"
rm "$set_dir"/{16,32,64,128,256}.png
iconutil -c icns -o "$app/Contents/Resources/icon.icns" "$set_dir"

# Apple Silicon refuses to start a program with no signature at all, so sign it without an identity.
codesign --force --sign - "$app"

# Otherwise the installer "upgrades" any copy it finds elsewhere on the disk instead of using /Applications.
pkgbuild --analyze --root "$work/root" "$work/component.plist"
plutil -replace 0.BundleIsRelocatable -bool NO "$work/component.plist"
pkgbuild --root "$work/root" --component-plist "$work/component.plist" --install-location /Applications \
  --identifier io.github.mydrift-user.bolt-explorer --version "$VERSION" "dist/$base.pkg"

ln -s /Applications "$work/root/Applications"
# hdiutil now and then fails on a busy CI machine with "Resource busy".
for try in 1 2 3; do
  hdiutil create -volname "Bolt Explorer" -srcfolder "$work/root" -format UDZO -ov "dist/$base.dmg" && exit 0
  sleep 10
done
exit 1

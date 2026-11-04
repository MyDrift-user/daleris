#!/bin/bash
# Installs the .pkg, starts the program, removes it, then checks the .dmg. Needs VERSION and NAME.
set -euo pipefail

base="dist/bolt-explorer-v$VERSION-$NAME"
app="/Applications/Bolt Explorer.app"
shot="$PWD/screens/$NAME.png"
mkdir -p screens /tmp/demo/Documents /tmp/demo/Pictures
echo hello > /tmp/demo/notes.txt

sudo installer -pkg "$base.pkg" -target /
codesign --verify "$app"

# macOS has no timeout command, so stop a program that hangs by hand.
"$app/Contents/MacOS/bolt-explorer" --screenshot "$shot" --path /tmp/demo &
pid=$!
( sleep 90; kill "$pid" 2>/dev/null ) &
watchdog=$!
wait "$pid"
kill "$watchdog" 2>/dev/null || true
test -s "$shot"
sudo rm -rf "$app"

volume=$(hdiutil attach -nobrowse -readonly "$base.dmg" | grep -o '/Volumes/.*' | head -1)
codesign --verify "$volume/Bolt Explorer.app"
test -L "$volume/Applications"
hdiutil detach "$volume"

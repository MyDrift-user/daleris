#!/bin/bash
# Updates the old/ build to the www/ build from a local server: the single file, the app installed by the .pkg, and the
# app dragged out of the .dmg. See ../test/build-two-versions.sh. Needs VERSION and NAME.
set -euo pipefail

new=99.0.0
base="old/daleris-v$VERSION-$NAME"
echo http://127.0.0.1:18765 > /tmp/daleris-update-test.txt
python3 -m http.server 18765 --bind 127.0.0.1 --directory www >/dev/null 2>&1 &
server=$!
trap 'kill $server' EXIT
sleep 2

check() { # program, version
  got=$("$1" --version)
  [ "$got" = "Daleris $2" ] || { echo "$1 is '$got', not $2"; exit 1; }
}

echo "=== A policy stops it"
work=$(mktemp -d)
cp "$base" "$work/daleris"
chmod +x "$work/daleris"
echo DisableUpdates=1 > "$work/policy.conf"
DALERIS_TEST_POLICY_FILE="$work/policy.conf" "$work/daleris" --update-now | grep -q 'managed by the administrator'
check "$work/daleris" "$VERSION"

echo "=== The single file"
"$work/daleris" --update-now
check "$work/daleris" "$new"

update_app() { # folder holding Daleris.app
  program="$1/Daleris.app/Contents/MacOS/daleris"
  check "$program" "$VERSION"
  "$program" --update-now
  check "$program" "$new"
  codesign --verify "$1/Daleris.app"
  if [ -e "$1/Daleris.old" ] || [ -e "$1/Daleris.new" ]; then
    echo "The update left files behind in $1"
    exit 1
  fi
}

echo "=== The app from the .dmg"
volume=$(hdiutil attach -nobrowse -readonly "$base.dmg" | grep -o '/Volumes/.*' | head -1)
mkdir -p "$work/Applications"
cp -R "$volume/Daleris.app" "$work/Applications/"
hdiutil detach "$volume"
update_app "$work/Applications"

echo "=== The app from the .pkg, updated by a normal user"
sudo installer -pkg "$base.pkg" -target /
update_app /Applications
sudo rm -rf /Applications/Daleris.app
echo "=== Everything passed"

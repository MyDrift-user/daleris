#!/bin/bash
# Updates the old/ build to the www/ build from a local server: the single file, the AppImage, and each package on a clean
# system. See ../test/build-two-versions.sh. Needs VERSION and NAME.
set -euo pipefail

new=99.0.0
base="old/daleris-v$VERSION-$NAME"
export HOME="$(mktemp -d)"
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

echo "=== AppImage"
export APPIMAGE_EXTRACT_AND_RUN=1
cp "$base.AppImage" "$work/daleris.AppImage"
chmod +x "$work/daleris.AppImage"
check "$work/daleris.AppImage" "$VERSION"
"$work/daleris.AppImage" --update-now
check "$work/daleris.AppImage" "$new"

package() { # image, package
  echo "=== $2 on $1"
  docker run --rm --network host -v "$PWD:/w" -w /w "$1" sh installer/linux/test-update-package.sh "$2" "$VERSION" "$new"
}
package debian:stable "$base.deb"
package ubuntu:22.04 "$base.deb"
package fedora:latest "$base.rpm"
package opensuse/tumbleweed "$base.rpm"
package archlinux:latest "$base.pkg.tar.zst"
echo "=== Everything passed"

#!/bin/bash
# For the update test: builds the release files twice with the test key, as this version into old/ and as 99.0.0 into
# www/, with a signed list next to them. Needs VERSION and NAME.
set -euo pipefail

package() {
  case "$RUNNER_OS" in
    Windows) pwsh -NoProfile -File installer/windows/package.ps1 ;;
    Linux) bash installer/linux/package.sh ;;
    macOS) bash installer/macos/package.sh ;;
  esac
}

cargo build --release --locked --features update-test
package
mv dist old

perl -i -pe 'if (!$done && s/^version = ".*"/version = "99.0.0"/) { $done = 1 }' Cargo.toml
cargo build --release --features update-test
VERSION=99.0.0 package
mv dist www

signer="old/daleris-v$VERSION-$NAME"
[ "$RUNNER_OS" = Windows ] && signer="$signer-portable.exe"
chmod +x "$signer"
cd www
if command -v sha256sum >/dev/null; then hash=(sha256sum); else hash=(shasum -a 256); fi
{ echo "version 99.0.0"; "${hash[@]}" -- * | sed -E 's/^([0-9a-f]+) +\*?/file \1 /'; } > latest.json
cd ..
"$signer" --test-sign www/latest.json www/latest.json.sig
test -s www/latest.json.sig
git checkout -- Cargo.toml Cargo.lock

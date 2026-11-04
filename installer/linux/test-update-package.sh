#!/bin/sh
# Runs inside a clean distro container: installs a package and has it update itself with the system's own package manager.
# A container has nobody to ask for a password, so here pkexec just runs the command.
# Usage: test-update-package.sh <package> <its version> <the new version>
set -eu

file="./$1"
case "$file" in
  *.deb) apt-get update -qq; apt-get install -y --no-install-recommends "$file" ;;
  *.rpm)
    if command -v zypper >/dev/null; then zypper -n --no-gpg-checks in --allow-unsigned-rpm "$file"
    else dnf install -y "$file"; fi ;;
  *.pkg.tar.zst) pacman -Sy --noconfirm; pacman -U --noconfirm "$file" ;;
esac

printf '#!/bin/sh\nexec "$@"\n' > /usr/local/bin/pkexec
chmod +x /usr/local/bin/pkexec
echo http://127.0.0.1:18765 > /tmp/daleris-update-test.txt
[ "$(daleris --version)" = "Daleris $2" ]
daleris --update-now
got=$(daleris --version)
[ "$got" = "Daleris $3" ] || { echo "It's '$got', not $3"; exit 1; }

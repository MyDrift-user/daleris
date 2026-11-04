#!/bin/sh
# Runs inside a clean distro container: installs a package, starts the program on a virtual
# screen, then uninstalls it. Usage: test-package.sh <package> <screenshot name>
set -eu

file="./$1"
shot="$PWD/screens/$2.png"
mkdir -p screens /tmp/demo/Documents /tmp/demo/Pictures
echo hello > /tmp/demo/notes.txt

# The package goes in first, so its own dependencies are what get installed.
case "$file" in
  *.AppImage) ;;
  *.deb) apt-get update -qq; apt-get install -y --no-install-recommends "$file" ;;
  *.rpm)
    if command -v zypper >/dev/null; then zypper -n --no-gpg-checks in --allow-unsigned-rpm "$file"
    else dnf install -y "$file"; fi ;;
  *.pkg.tar.zst) pacman -Sy --noconfirm; pacman -U --noconfirm "$file" ;;
esac

if command -v apt-get >/dev/null; then
  [ -n "${file##*.AppImage}" ] || apt-get update -qq
  apt-get install -y --no-install-recommends xvfb libgl1-mesa-dri
elif command -v zypper >/dev/null; then zypper -n in xorg-x11-server-Xvfb Mesa-dri
elif command -v dnf >/dev/null; then dnf install -y xorg-x11-server-Xvfb mesa-dri-drivers
else pacman -S --noconfirm xorg-server-xvfb mesa
fi

Xvfb :99 -screen 0 1280x1024x24 &
sleep 3
export DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1

case "$file" in
  *.AppImage) export APPIMAGE_EXTRACT_AND_RUN=1; program="$file" ;;
  *) program=daleris ;;
esac
timeout 90 "$program" --screenshot "$shot" --path /tmp/demo
test -s "$shot"

case "$file" in
  *.AppImage) exit 0 ;;
  *.deb) apt-get remove -y daleris ;;
  *.rpm)
    if command -v zypper >/dev/null; then zypper -n rm daleris
    else dnf remove -y daleris; fi ;;
  *.pkg.tar.zst) pacman -R --noconfirm daleris ;;
esac
if command -v daleris >/dev/null; then
  echo "daleris is still there after uninstalling"
  exit 1
fi

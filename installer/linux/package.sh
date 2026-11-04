#!/bin/bash
# Builds the Linux release files into dist/. Needs VERSION and NAME (linux-x64 or linux-arm64).
set -euo pipefail

base="daleris-v$VERSION-$NAME"
machine=$(uname -m)
case "$NAME" in
  linux-x64) export PKG_ARCH=amd64 nfpm_arch=x86_64 ;;
  linux-arm64) export PKG_ARCH=arm64 nfpm_arch=arm64 ;;
  *) echo "Unknown target $NAME"; exit 1 ;;
esac

mkdir -p dist
cp target/release/daleris "dist/$base"
tar czf "dist/$base.tar.gz" -C target/release daleris -C "$PWD" LICENSE

# Native packages
tools=$(mktemp -d)
curl -fsSL "https://github.com/goreleaser/nfpm/releases/download/v2.48.0/nfpm_2.48.0_Linux_$nfpm_arch.tar.gz" | tar xz -C "$tools" nfpm
"$tools/nfpm" package --config installer/linux/nfpm.yaml --packager deb --target "dist/$base.deb"
"$tools/nfpm" package --config installer/linux/nfpm.yaml --packager rpm --target "dist/$base.rpm"
# Arch Linux itself only exists for x64.
if [ "$NAME" = linux-x64 ]; then
  "$tools/nfpm" package --config installer/linux/nfpm.yaml --packager archlinux --target "dist/$base.pkg.tar.zst"
fi

# AppImage. Graphics libraries stay out because they have to match the PC's drivers. The
# others are found through the program's run path instead of LD_LIBRARY_PATH, which would
# leak into every program a file explorer starts.
app=$(mktemp -d)/AppDir
mkdir -p "$app/usr/bin" "$app/usr/lib"
cp target/release/daleris "$app/usr/bin/"
patchelf --set-rpath '$ORIGIN/../lib' "$app/usr/bin/daleris"
for lib in libxkbcommon.so.0 libxkbcommon-x11.so.0 libxcb-xkb.so.1 libXcursor.so.1 libXi.so.6 libXrender.so.1 libXfixes.so.3; do
  cp -L "/usr/lib/$machine-linux-gnu/$lib" "$app/usr/lib/"
  patchelf --set-rpath '$ORIGIN' "$app/usr/lib/$lib"
done
cp installer/linux/daleris.desktop "$app/"
cp LICENSE "$app/usr/"
cp assets/logo.png "$app/daleris.png"
ln -s usr/bin/daleris "$app/AppRun"
curl -fsSL -o "$tools/appimagetool" "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-$machine.AppImage"
chmod +x "$tools/appimagetool"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$machine" "$tools/appimagetool" --no-appstream "$app" "dist/$base.AppImage"
chmod +x "dist/$base" "dist/$base.AppImage"

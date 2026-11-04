#!/bin/bash
# Runs the built program on Linux or macOS: real folders, the settings file, and whether a crash
# reaches the log. Linux needs Xvfb. Screenshots end up in screens/.
set -uo pipefail

bin="$PWD/target/release/bolt-explorer"
out="$PWD/screens"
mkdir -p "$out"
export HOME="$(mktemp -d)"
unset XDG_CONFIG_HOME XDG_STATE_HOME
if [ "$(uname)" = Darwin ]; then
  log="$HOME/Library/Logs/Bolt Explorer/bolt-explorer.log"
  config="$HOME/Library/Application Support/Bolt Explorer/settings.txt"
else
  log="$HOME/.local/state/bolt-explorer/bolt-explorer.log"
  config="$HOME/.config/bolt-explorer/settings.txt"
  export LIBGL_ALWAYS_SOFTWARE=1
fi
failed=0
fail() { echo "FAILED: $1"; failed=1; }

# macOS has no timeout command.
run() {
  "$bin" "$@" &
  local pid=$!
  ( sleep 90; kill -9 "$pid" 2>/dev/null ) &
  local watchdog=$!
  wait "$pid"
  local code=$?
  kill "$watchdog" 2>/dev/null
  return $code
}

# Every kind of file the icons know, a known folder, names that are hard to handle, and a hidden file.
demo="$HOME/demo"
mkdir -p "$demo/Folder" "$demo/.hidden folder" "$HOME/Documents" "$HOME/Pictures" "$HOME/Downloads"
for f in notes.txt report.pdf letter.docx sheet.xlsx slides.pptx data.csv photo.jpg clip.mp4 song.mp3 main.rs archive.zip tool.exe unknown.xyz .hidden "ünï çödé €.txt" "名前.pdf"; do
  echo hello > "$demo/$f"
done
printf 'x' > "$demo/$(printf 'bad\xffname.txt')" || true

shots=(
  "details|--path $demo"
  "icons|--path $demo --icons"
  "search|--path $demo --query .txt"
  "home|--path $HOME"
  "settings-appearance|--settings appearance --path $demo"
  "settings-files|--settings files --path $demo"
  "system-folder|--path /usr/lib --query lib"
  "root|--path /"
)
for s in "${shots[@]}"; do
  name=${s%%|*}
  # shellcheck disable=SC2086
  run --screenshot "$out/$name.png" ${s#*|} || fail "$name exited with $?"
  [ -s "$out/$name.png" ] || fail "$name made no screenshot"
done

[ -s "$log" ] || fail "no log file at $log"
grep -q 'graphics: ' "$log" || fail "the log doesn't name the graphics driver"
grep -q ' exited$' "$log" || fail "the log has no clean exit"
if grep -q 'CRASH' "$log"; then fail "a normal run crashed"; fi

# The settings file gets written, and read back.
BOLT_TEST_SETTINGS=1 run --screenshot "$out/settings-write.png" --path "$demo" || fail "settings run exited with $?"
[ -s "$config" ] || fail "no settings file at $config"
# By hand: comfortable rows, large icons and the old Windows icon setting, which must not leave blank icons.
printf 'density=2\nview=1\niconpack=system\n' >> "$config"
BOLT_TEST_SETTINGS=1 run --screenshot "$out/settings-read.png" --path "$demo" || fail "second settings run exited with $?"
grep -q '^density=2' "$config" || fail "a setting edited by hand was lost"

# Crashes must reach the log.
BOLT_TEST_CRASH=panic run --path "$demo" && fail "the test panic didn't stop the program"
grep -q "CRASH panic on thread 'main'" "$log" && grep -q 'test panic from BOLT_TEST_CRASH' "$log" || fail "no panic in the log"
BOLT_TEST_CRASH=signal run --path "$demo" && fail "the test signal didn't stop the program"
grep -q 'CRASH fatal signal SIGSEGV' "$log" || fail "no signal in the log"

echo "--- log"
cat "$log"
echo "--- settings"
cat "$config"
cp "$log" "$out/bolt-explorer.log"
exit $failed

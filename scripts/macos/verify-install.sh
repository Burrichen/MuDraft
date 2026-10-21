#!/usr/bin/env bash
# Verifies the macOS DMG on a real Mac (CI runner or a throwaway machine). It installs
# MuDraft into /Applications and launches it for the current user, so never run it on a
# Mac whose MuDraft profile matters.
#
# Checks: DMG integrity and layout (MuDraft.app + Applications shortcut), bundle metadata
# and icon, a universal (arm64 + x86_64) executable, the code signature (ad-hoc unless a
# Developer ID build), no test hooks, first launch through LaunchServices (as from Finder),
# the production data folder (the development override is ignored), persistence across a
# restart, an upgrade that keeps the profile and artwork, the running architecture, and
# an uninstall (deleting the app) that keeps the profile.
#
# Usage: verify-install.sh <release.dmg> <baseline-app.zip> <version> <old-version> <shots-dir> <signed:true|false>
set -euo pipefail

DMG=$1
BASELINE=$2
VERSION=$3
OLD_VERSION=$4
SHOTS=$5
SIGNED=$6

BUNDLE_ID=app.mudraft.desktop
APP=/Applications/MuDraft.app
DATA="$HOME/Library/Application Support/$BUNDLE_ID"
DB="$DATA/mudraft.sqlite3"
ARTWORK="$DATA/artwork"
IGNORED="${RUNNER_TEMP:-/tmp}/mudraft-dev-override"
MOUNT="$(mktemp -d)/MuDraft-dmg"
mkdir -p "$SHOTS"

check() {
  local what=$1
  shift
  if "$@"; then echo "  ok: $what"; else echo "VERIFY FAILED: $what" >&2; exit 1; fi
}
sql() { sqlite3 "$DB" "$1"; }
plist() { /usr/libexec/PlistBuddy -c "Print :$2" "$1/Contents/Info.plist"; }
executable() { echo "$1/Contents/MacOS/$(plist "$1" CFBundleExecutable)"; }
cleanup() { hdiutil detach "$MOUNT" -quiet 2>/dev/null || true; }
trap cleanup EXIT

wait_for_db() {
  for _ in $(seq 180); do [ -f "$DB" ] && return 0; sleep 0.5; done
  return 1
}

running() { pgrep -f "$1" >/dev/null; }

# Launch, wait for the profile, check it stays up, screenshot, quit. $2 picks how:
# `open` goes through LaunchServices like a Finder double-click; `exec` runs the binary
# with the development override set (a release build must ignore it); `rosetta` runs
# the x86_64 slice under Rosetta on Apple Silicon.
launch() {
  local label=$1 how=$2 exe
  exe=$(executable "$APP")
  echo "== Launch ($label, $how)"
  case $how in
    open) open -n "$APP" ;;
    exec) MUDRAFT_DATA_DIR="$IGNORED" "$exe" >/dev/null 2>&1 & ;;
    rosetta) MUDRAFT_DATA_DIR="$IGNORED" arch -x86_64 "$exe" >/dev/null 2>&1 & ;;
  esac
  check "profile database exists at $DB" wait_for_db
  sleep 8
  # `arch -x86_64` refuses to start anything but the x86_64 slice, so running = executed.
  check "app is still running after start-up" running "$exe"
  # Bounded: on a headless runner screen capture may wait for a permission that never comes.
  perl -e 'alarm 20; exec @ARGV' screencapture -x "$SHOTS/$label.png" 2>/dev/null ||
    echo "  (no screenshot: screen capture unavailable)"
  pkill -TERM -f "$exe" || true
  for _ in $(seq 30); do running "$exe" || break; sleep 0.5; done
  if running "$exe"; then pkill -KILL -f "$exe" || true; sleep 1; fi
  check "release build ignored the development data override" test ! -e "$IGNORED"
  check "database integrity check passes" test "$(sql 'PRAGMA quick_check')" = ok
}

install_app() {
  local from=$1 expect=$2
  rm -rf "$APP"
  ditto "$from" "$APP"
  check "installed version is $expect" test "$(plist "$APP" CFBundleShortVersionString)" = "$expect"
}

echo "Runner: $(sw_vers -productName) $(sw_vers -productVersion), $(uname -m)"
check "no previous install on this machine" test ! -e "$APP"
check "no previous profile on this machine" test ! -e "$DATA"

# 1. The disk image itself.
echo "== Disk image"
check "DMG checksum verifies" hdiutil verify -quiet "$DMG"
mkdir -p "$MOUNT"
hdiutil attach "$DMG" -nobrowse -readonly -noautoopen -mountpoint "$MOUNT" -quiet
NEW="$MOUNT/MuDraft.app"
check "DMG contains MuDraft.app" test -d "$NEW"
check "DMG contains an Applications shortcut" test "$(readlink "$MOUNT/Applications")" = /Applications
check "bundle identifier is $BUNDLE_ID" test "$(plist "$NEW" CFBundleIdentifier)" = "$BUNDLE_ID"
check "bundle version is $VERSION" test "$(plist "$NEW" CFBundleShortVersionString)" = "$VERSION"
check "minimum macOS is 12.0" test "$(plist "$NEW" LSMinimumSystemVersion)" = 12.0
ICON="$NEW/Contents/Resources/$(plist "$NEW" CFBundleIconFile)"
[[ $ICON == *.icns ]] || ICON="$ICON.icns"
check "app icon is bundled" test -s "$ICON"
ARCHS=$(lipo -archs "$(executable "$NEW")")
echo "  executable architectures: $ARCHS"
check "executable contains arm64" grep -qw arm64 <<<"$ARCHS"
check "executable contains x86_64" grep -qw x86_64 <<<"$ARCHS"
check "code signature is valid" codesign --verify --deep --strict "$NEW"
SIGNATURE=$(codesign -dv "$NEW" 2>&1)
echo "$SIGNATURE" | grep -E '^(Signature|Authority|TeamIdentifier|Format)=' | sed 's/^/  /'
if [ "$SIGNED" = true ]; then
  check "signed with a Developer ID" grep -q '^Authority=Developer ID Application' <<<"$SIGNATURE"
else
  check "ad-hoc signed (no Developer ID)" grep -q '^Signature=adhoc' <<<"$SIGNATURE"
fi
echo "  Gatekeeper assessment (expected to reject unless notarized):"
spctl --assess --type execute -vv "$NEW" 2>&1 | sed 's/^/    /' || true
for needle in wdio MUDRAFT_E2E TAURI_WEBDRIVER_PORT; do
  check "app binary has no '$needle' test hook" sh -c "! grep -aq '$needle' '$(executable "$NEW")'"
done
check "no fixtures, test data, or databases in the bundle" \
  sh -c "! find '$NEW' -type f | grep -Eiq '\.(sqlite3?|db|csv|mudraft)$|fixture|e2e'"

# 2. Older version: install, first launch through LaunchServices.
echo "== Install $OLD_VERSION (upgrade baseline)"
OLD_DIR=$(mktemp -d)
ditto -x -k "$BASELINE" "$OLD_DIR"
install_app "$OLD_DIR/MuDraft.app" "$OLD_VERSION"
launch first-run open
check "database is at schema 12 or later" test "$(sql 'PRAGMA user_version')" -ge 12

# 3. Seed user data while the app is closed, then restart.
echo "== Seed user data"
sql "INSERT INTO tag (id, name, color) VALUES ('0190f5c3-0000-7000-8000-00000000beef', 'Upgrade marker', '#22d3ee')"
mkdir -p "$ARTWORK"
base64 -d >"$ARTWORK/marker.png" <<<'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=='
MARKER="SELECT COUNT(*) FROM tag WHERE name = 'Upgrade marker'"
launch restart exec
check "profile kept after restart" test "$(sql "$MARKER")" = 1

# 4. Upgrade: replace the app from the DMG, as dragging onto Applications does.
echo "== Upgrade to $VERSION"
install_app "$NEW" "$VERSION"
launch after-upgrade open
check "profile kept across upgrade" test "$(sql "$MARKER")" = 1
check "artwork kept across upgrade" test -s "$ARTWORK/marker.png"
check "Listen ASAP default is mustard" test "$(sql "SELECT color FROM tag WHERE builtin_key = 'listen_asap'")" = '#d4a017'

# 5. The other architecture, where this Mac can run it.
if [ "$(uname -m)" = arm64 ]; then
  if arch -x86_64 /usr/bin/true 2>/dev/null; then
    launch rosetta-x86_64 rosetta
    check "profile kept when the Intel slice runs" test "$(sql "$MARKER")" = 1
  else
    echo "  NOTE: Rosetta isn't installed; the x86_64 slice was inspected but not run here."
  fi
fi

# 6. Uninstall: deleting the app keeps the profile.
echo "== Uninstall"
rm -rf "$APP"
check "app removed" test ! -e "$APP"
check "profile database kept after uninstall" test -f "$DB"
check "profile contents kept after uninstall" test "$(sql "$MARKER")" = 1
check "artwork kept after uninstall" test -s "$ARTWORK/marker.png"
echo "All macOS install checks passed on $(uname -m)."

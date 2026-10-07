#!/bin/sh
set -eu
umask 077

REPO="GitAashishG/howdo"
INSTALL_DIR="${HOWDO_INSTALL_DIR:-/usr/local/bin}"
TEMP_DIR=""
STAGED=""
USE_SUDO=false

as_owner() {
    if [ "$USE_SUDO" = true ]; then sudo "$@"; else "$@"; fi
}

cleanup() {
    if [ -n "$STAGED" ]; then as_owner rm -f "$STAGED" || true; fi
    if [ -n "$TEMP_DIR" ]; then rm -rf "$TEMP_DIR" || true; fi
}
trap cleanup 0
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

case "$INSTALL_DIR" in
    /*) ;;
    *) echo "HOWDO_INSTALL_DIR must be an absolute path." >&2; exit 1 ;;
esac

case "$(uname -s):$(uname -m)" in
    Darwin:arm64|Darwin:aarch64) TARGET="aarch64-apple-darwin" ;;
    Darwin:x86_64) TARGET="x86_64-apple-darwin" ;;
    Linux:x86_64|Linux:amd64) TARGET="x86_64-unknown-linux-gnu" ;;
    *) echo "No prebuilt binary for this OS/architecture. Build from source or use install.ps1 on Windows." >&2; exit 1 ;;
esac

download() {
    curl --proto '=https' --proto-redir '=https' -fsSL \
        --connect-timeout 10 --max-time 120 --retry 2 -o "$2" "$1"
}

TEMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/howdo-install.XXXXXX")
download "https://api.github.com/repos/$REPO/releases/latest" "$TEMP_DIR/release.json"
TAG=$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$TEMP_DIR/release.json")
if ! printf '%s\n' "$TAG" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "Could not determine a stable release version." >&2
    exit 1
fi
ASSET="howdo-$TARGET"
BASE="https://github.com/$REPO/releases/download/$TAG"
echo "Downloading howdo $TAG for $TARGET..."
download "$BASE/$ASSET" "$TEMP_DIR/howdo"
download "$BASE/SHA256SUMS" "$TEMP_DIR/SHA256SUMS"
EXPECTED=$(awk -v name="$ASSET" '$2 == name || $2 == "*" name { print tolower($1) }' "$TEMP_DIR/SHA256SUMS")
if [ "${#EXPECTED}" -ne 64 ] || ! printf '%s\n' "$EXPECTED" | grep -Eq '^[0-9a-f]+$'; then
    echo "Missing, duplicate, or invalid checksum. Nothing was installed." >&2
    exit 1
fi
if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL=$(sha256sum "$TEMP_DIR/howdo" | awk '{ print $1 }')
elif command -v shasum >/dev/null 2>&1; then
    ACTUAL=$(shasum -a 256 "$TEMP_DIR/howdo" | awk '{ print $1 }')
else
    echo "SHA-256 verification requires sha256sum or shasum. Nothing was installed." >&2
    exit 1
fi
if [ "$ACTUAL" != "$EXPECTED" ] || [ ! -s "$TEMP_DIR/howdo" ]; then
    echo "Checksum mismatch or empty binary. Nothing was installed." >&2
    exit 1
fi

if [ ! -d "$INSTALL_DIR" ]; then
    mkdir -p "$INSTALL_DIR" || {
        echo "Create $INSTALL_DIR first, or set HOWDO_INSTALL_DIR to a user-writable directory." >&2
        exit 1
    }
fi
if [ ! -w "$INSTALL_DIR" ]; then
    command -v sudo >/dev/null 2>&1 || { echo "Installation requires permission to write $INSTALL_DIR." >&2; exit 1; }
    USE_SUDO=true
fi
STAGED=$(as_owner mktemp "$INSTALL_DIR/.howdo.XXXXXX")
as_owner cp "$TEMP_DIR/howdo" "$STAGED"
as_owner chmod 755 "$STAGED"
# Stage on the destination filesystem so replacing an existing binary is atomic.
as_owner mv -f "$STAGED" "$INSTALL_DIR/howdo"
STAGED=""
echo "Installed $TAG to $INSTALL_DIR/howdo (SHA-256 verified)."
case ":${PATH:-}:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "Add $INSTALL_DIR to your PATH." ;;
esac
echo "Run 'howdo /config' to configure a provider. No startup files were changed."

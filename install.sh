#!/bin/sh
set -e

REPO="GitAashishG/howdo"
INSTALL_DIR="/usr/local/bin"
BINARY="howdo"

get_arch() {
    arch=$(uname -m)
    case "$arch" in
        x86_64|amd64)  echo "x86_64" ;;
        arm64|aarch64) echo "aarch64" ;;
        *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
    esac
}

get_os() {
    os=$(uname -s)
    case "$os" in
        Linux)  echo "unknown-linux-gnu" ;;
        Darwin) echo "apple-darwin" ;;
        *) echo "Unsupported OS: $os (use the Windows .exe from the releases page)" >&2; exit 1 ;;
    esac
}

ARCH=$(get_arch)
OS=$(get_os)
TARGET="${ARCH}-${OS}"

# Get latest release tag
LATEST=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')

if [ -z "$LATEST" ]; then
    echo "Error: Could not determine latest release." >&2
    exit 1
fi

URL="https://github.com/${REPO}/releases/download/${LATEST}/howdo-${TARGET}"

echo "Downloading howdo ${LATEST} for ${TARGET}..."
curl -fsSL -o "/tmp/${BINARY}" "$URL"
chmod +x "/tmp/${BINARY}"

echo "Installing to ${INSTALL_DIR}/${BINARY} (may need sudo)..."
if [ -w "$INSTALL_DIR" ]; then
    mv "/tmp/${BINARY}" "${INSTALL_DIR}/${BINARY}"
else
    sudo mv "/tmp/${BINARY}" "${INSTALL_DIR}/${BINARY}"
fi

echo "Installed! Run 'howdo /config' to set up your LLM provider."
echo ""
echo "Tip: The config wizard will offer to set up a shell alias for you."
echo "     Or add it manually:  alias q='noglob howdo'  # ~/.zshrc"

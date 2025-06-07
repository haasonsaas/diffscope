#!/bin/sh
# DiffScope Installation Script
# This script detects your platform and downloads the appropriate binary

set -e

REPO="haasonsaas/diffscope"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
BINARY_NAME="diffscope"

# Detect OS and architecture
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# Map to Rust target triples
case "$OS" in
    linux)
        case "$ARCH" in
            x86_64)
                TARGET="x86_64-unknown-linux-musl"
                ;;
            aarch64)
                TARGET="aarch64-unknown-linux-gnu"
                ;;
            *)
                echo "Unsupported architecture: $ARCH"
                exit 1
                ;;
        esac
        ;;
    darwin)
        case "$ARCH" in
            x86_64)
                TARGET="x86_64-apple-darwin"
                ;;
            arm64)
                TARGET="aarch64-apple-darwin"
                ;;
            *)
                echo "Unsupported architecture: $ARCH"
                exit 1
                ;;
        esac
        ;;
    *)
        echo "Unsupported OS: $OS"
        exit 1
        ;;
esac

# Get latest release
echo "Fetching latest release..."
LATEST_RELEASE=$(curl -s "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')

if [ -z "$LATEST_RELEASE" ]; then
    echo "Failed to fetch latest release"
    exit 1
fi

echo "Latest release: $LATEST_RELEASE"

# Download URL
DOWNLOAD_URL="https://github.com/$REPO/releases/download/$LATEST_RELEASE/diffscope-$TARGET"

# Download binary
echo "Downloading $BINARY_NAME for $TARGET..."
TMP_FILE=$(mktemp)
curl -L "$DOWNLOAD_URL" -o "$TMP_FILE"

# Make executable
chmod +x "$TMP_FILE"

# Check if we need sudo
if [ -w "$INSTALL_DIR" ]; then
    mv "$TMP_FILE" "$INSTALL_DIR/$BINARY_NAME"
else
    echo "Installing to $INSTALL_DIR requires sudo access"
    sudo mv "$TMP_FILE" "$INSTALL_DIR/$BINARY_NAME"
fi

# Verify installation
if command -v diffscope >/dev/null 2>&1; then
    echo "✅ DiffScope installed successfully!"
    echo "Version: $(diffscope --version)"
else
    echo "⚠️  Installation completed but diffscope not found in PATH"
    echo "You may need to add $INSTALL_DIR to your PATH"
fi
#!/usr/bin/env sh
# sivtr installer (Linux/macOS/WSL/Git-Bash) - https://github.com/Ariestar/sivtr
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Ariestar/sivtr/main/install.sh | sh
#   SIVTR_VERSION=v0.2.5 sh install.sh        # pin a version
#   SIVTR_INSTALL_DIR=/opt/bin sh install.sh  # override install location
#
# Windows users: prefer install.ps1 instead:
#   irm https://raw.githubusercontent.com/Ariestar/sivtr/main/install.ps1 | iex

set -e

REPO="Ariestar/sivtr"
BINARY_NAME="sivtr"
INSTALL_DIR="${SIVTR_INSTALL_DIR:-$HOME/.local/bin}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { printf "${GREEN}[INFO]${NC} %s\n" "$1"; }
warn()  { printf "${YELLOW}[WARN]${NC} %s\n" "$1"; }
error() { printf "${RED}[ERROR]${NC} %s\n" "$1"; exit 1; }

detect_platform() {
    KERNEL="$(uname -s)"
    ARCH_RAW="$(uname -m)"

    case "$ARCH_RAW" in
        x86_64|amd64)   ARCH="x86_64" ;;
        arm64|aarch64)  ARCH="aarch64" ;;
        *)              error "Unsupported architecture: $ARCH_RAW" ;;
    esac

    case "$KERNEL" in
        Linux*)                 PLATFORM="linux" ;;
        Darwin*)                PLATFORM="darwin" ;;
        MINGW*|MSYS*|CYGWIN*)   PLATFORM="windows" ;;
        *)                      error "Unsupported operating system: $KERNEL" ;;
    esac
}

# Map (platform, arch) -> release asset name + archive format.
# Assets are produced by .github/workflows/release.yml and named
# sivtr-<version>-<asset>.<ext>; each archive contains a top-level
# sivtr-<version>-<asset>/ directory holding the binary, README.md and LICENSE.
resolve_target() {
    case "$PLATFORM" in
        linux)
            case "$ARCH" in
                x86_64) ASSET="linux-x64-musl"; EXT="tar.gz"; BIN_EXT="" ;;
                *) error "No prebuilt Linux binary for $ARCH. Install with: cargo binstall sivtr" ;;
            esac
            ;;
        darwin)
            case "$ARCH" in
                aarch64) ASSET="macos"; EXT="tar.gz"; BIN_EXT="" ;;
                *) error "No prebuilt macOS Intel binary. Install with: cargo binstall sivtr" ;;
            esac
            ;;
        windows)
            case "$ARCH" in
                x86_64) ASSET="windows-x64"; EXT="zip"; BIN_EXT=".exe" ;;
                *) error "No prebuilt Windows binary for $ARCH. Install with: cargo binstall sivtr" ;;
            esac
            ;;
    esac
}

get_latest_version() {
    VERSION=$(curl -sI "https://github.com/${REPO}/releases/latest" \
        | grep -i '^location:' \
        | sed -E 's|.*/tag/([^[:space:]]+).*|\1|' \
        | tr -d '\r')

    if [ -z "$VERSION" ]; then
        warn "Redirect lookup failed, falling back to GitHub API..."
        VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name":' \
            | sed -E 's/.*"([^"]+)".*/\1/')
    fi

    if [ -z "$VERSION" ]; then
        error "Failed to determine latest version. Set SIVTR_VERSION=vX.Y.Z or install with: cargo binstall sivtr"
    fi
}

download() {
    DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${BINARY_NAME}-${VERSION}-${ASSET}.${EXT}"
    TEMP_DIR=$(mktemp -d)
    ARCHIVE="${TEMP_DIR}/sivtr.${EXT}"

    info "Downloading $DOWNLOAD_URL"
    if ! curl -fsSL "$DOWNLOAD_URL" -o "$ARCHIVE"; then
        error "Download failed. Try: cargo binstall sivtr"
    fi
}

verify_archive_safety() {
    # Refuse archives whose entries use absolute or traversal paths.
    if [ "$EXT" = "zip" ]; then
        if unzip -l "$ARCHIVE" 2>/dev/null | awk 'NR>3 {print $4}' | grep -qE '^/|(^|/)\.\.(/|$)'; then
            error "Archive contains unsafe paths — refusing to extract"
        fi
    else
        if tar -tzf "$ARCHIVE" | grep -qE '^/|(^|/)\.\.(/|$)'; then
            error "Archive contains unsafe paths — refusing to extract"
        fi
    fi
}

extract_and_install() {
    EXTRACT_DIR="${TEMP_DIR}/out"
    mkdir -p "$EXTRACT_DIR"

    if [ "$EXT" = "zip" ]; then
        if command -v unzip >/dev/null 2>&1; then
            unzip -q "$ARCHIVE" -d "$EXTRACT_DIR"
        else
            error "unzip is required on Windows-like shells. Install it, or use install.ps1."
        fi
    else
        tar -xzf "$ARCHIVE" -C "$EXTRACT_DIR"
    fi

    BINARY_PATH=$(find "$EXTRACT_DIR" -type f -name "${BINARY_NAME}${BIN_EXT}" 2>/dev/null | head -n 1)
    if [ -z "$BINARY_PATH" ]; then
        error "Could not find ${BINARY_NAME}${BIN_EXT} in the archive."
    fi

    mkdir -p "$INSTALL_DIR"
    mv "$BINARY_PATH" "${INSTALL_DIR}/${BINARY_NAME}${BIN_EXT}"
    [ -z "$BIN_EXT" ] && chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

    rm -rf "$TEMP_DIR"
    info "Installed ${BINARY_NAME}${BIN_EXT} to ${INSTALL_DIR}/${BINARY_NAME}${BIN_EXT}"
}

verify() {
    if command -v "$BINARY_NAME" >/dev/null 2>&1; then
        info "Verification: $($BINARY_NAME --version)"
    else
        warn "${BINARY_NAME} installed but not on PATH."
        warn "  Add to your shell profile: export PATH=\"${INSTALL_DIR}:\$PATH\""
    fi
}

main() {
    info "Installing $BINARY_NAME"

    detect_platform
    resolve_target

    if [ -n "$SIVTR_VERSION" ]; then
        VERSION="$SIVTR_VERSION"
        info "Using pinned version: $VERSION"
    else
        get_latest_version
    fi

    info "Platform: $PLATFORM $ARCH · Asset: sivtr-${VERSION}-${ASSET}.${EXT}"

    download
    verify_archive_safety
    extract_and_install
    verify

    echo ""
    info "Done. Run 'sivtr doctor' to verify your setup."
}

main

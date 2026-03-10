#!/bin/sh
set -e

DOWNLOADS_BASE_URL="https://downloads.trycaboose.dev"
INSTALL_DIR="/usr/local/bin"
BINARY_NAME="caboose"

main() {
    # Detect OS
    os="$(uname -s)"
    case "$os" in
        Darwin) os_name="apple-darwin" ;;
        Linux)  os_name="unknown-linux-musl" ;;
        *)      echo "Error: Unsupported OS: $os"; exit 1 ;;
    esac

    # Detect architecture
    arch="$(uname -m)"
    case "$arch" in
        arm64|aarch64) arch_name="aarch64" ;;
        x86_64)        arch_name="x86_64" ;;
        *)             echo "Error: Unsupported architecture: $arch"; exit 1 ;;
    esac

    target="${arch_name}-${os_name}"
    artifact="${BINARY_NAME}-${target}.tar.xz"

    # Resolve version
    if [ -n "$1" ] && echo "$1" | grep -q "^v"; then
        version="$1"
    else
        echo "Fetching latest version..."
        version="$(curl -fsSL "${DOWNLOADS_BASE_URL}/latest.txt")"
    fi

    echo "Installing ${BINARY_NAME} ${version} (${target})..."

    # Create temp directory
    tmp_dir="$(mktemp -d)"
    trap 'rm -rf "$tmp_dir"' EXIT

    # Download artifact and checksums
    curl -fsSL "${DOWNLOADS_BASE_URL}/${version}/${artifact}" -o "${tmp_dir}/${artifact}"
    curl -fsSL "${DOWNLOADS_BASE_URL}/${version}/checksums.txt" -o "${tmp_dir}/checksums.txt"

    # Verify checksum
    expected_checksum="$(grep "${artifact}" "${tmp_dir}/checksums.txt" | awk '{print $1}')"
    if [ -z "$expected_checksum" ]; then
        echo "Error: Checksum not found for ${artifact}"
        exit 1
    fi

    if command -v sha256sum >/dev/null 2>&1; then
        actual_checksum="$(sha256sum "${tmp_dir}/${artifact}" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
        actual_checksum="$(shasum -a 256 "${tmp_dir}/${artifact}" | awk '{print $1}')"
    else
        echo "Warning: No sha256 tool found, skipping checksum verification"
        actual_checksum="$expected_checksum"
    fi

    if [ "$actual_checksum" != "$expected_checksum" ]; then
        echo "Error: Checksum verification failed"
        echo "  Expected: ${expected_checksum}"
        echo "  Got:      ${actual_checksum}"
        exit 1
    fi

    # Extract
    tar -xJf "${tmp_dir}/${artifact}" -C "${tmp_dir}"

    # Install
    if [ -w "$INSTALL_DIR" ]; then
        cp "${tmp_dir}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    else
        echo "Installing to ${INSTALL_DIR} (requires sudo)..."
        sudo cp "${tmp_dir}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
    fi
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

    # macOS: strip quarantine attribute
    if [ "$os" = "Darwin" ]; then
        xattr -d com.apple.quarantine "${INSTALL_DIR}/${BINARY_NAME}" 2>/dev/null || true
    fi

    echo ""
    echo "caboose ${version} installed to ${INSTALL_DIR}/${BINARY_NAME}"
    echo ""
    echo "Run 'caboose' to get started."
    echo "Run 'caboose update' to update in the future."
}

main "$@"

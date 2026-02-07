#!/usr/bin/env bash
#
# run_demo1.sh — Quick demo of secretfs
#
# Builds the project, mounts a demo directory through secretfs, and
# lets you explore the filtered view in /tmp/filtered.
#
# The source directory (demo/stuff-original/) contains files with embedded
# secrets. Through the mount at /tmp/filtered, those secrets are replaced
# with stable placeholders like <|SECRET:0001|>.
#
# Usage:
#   ./run_demo1.sh          # build & mount
#   cat /tmp/filtered/foo.txt   # (in another terminal) see placeholders
#   umount /tmp/filtered    # unmount when done
#
set -euo pipefail

# ── Platform Detection ──────────────────────────────────────────────
OS="$(uname -s)"

# ── Colors ────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

SOURCE_DIR="./demo/stuff-original"
MOUNT_DIR="/tmp/filtered"
CONFIG="./demo/secrets.yml"
BINARY="./target/release/secretfs"

# ── Preflight checks ─────────────────────────────────────────────────
echo -e "${CYAN}${BOLD}── secretfs demo ──${RESET}"
echo

# Check for FUSE
if [[ "$OS" == "Darwin" ]]; then
    # macOS uses macFUSE, usually doesn't have fusermount in PATH
    echo -e "${GREEN}✓${RESET} macOS detected (assuming macFUSE is installed)"
else
    if ! command -v fusermount3 &>/dev/null && ! command -v fusermount &>/dev/null; then
        echo -e "${RED}✗ fusermount3 (or fusermount) not found.${RESET}"
        echo -e "  Install FUSE first:  ${YELLOW}apt-get install fuse3${RESET}  (Debian/Ubuntu)"
        exit 1
    fi
    echo -e "${GREEN}✓${RESET} FUSE utilities found"
fi

# Check for Rust toolchain
if ! command -v cargo &>/dev/null; then
    echo -e "${RED}✗ cargo not found. Install the Rust toolchain first.${RESET}"
    exit 1
fi
echo -e "${GREEN}✓${RESET} Rust toolchain found"

# Check that the secrets config exists
if [[ ! -f "$CONFIG" ]]; then
    echo -e "${RED}✗ Secrets config not found: ${CONFIG}${RESET}"
    exit 1
fi
echo -e "${GREEN}✓${RESET} Secrets config: ${CYAN}${CONFIG}${RESET}"

# Check that the source directory exists
if [[ ! -d "$SOURCE_DIR" ]]; then
    echo -e "${YELLOW}⚠ Source directory missing — creating ${SOURCE_DIR}${RESET}"
    mkdir -p "$SOURCE_DIR"
fi
echo -e "${GREEN}✓${RESET} Source directory: ${CYAN}${SOURCE_DIR}${RESET}"

# Make sure the mount point isn't already in use
IS_MOUNTED=0
if [[ "$OS" == "Darwin" ]]; then
    if mount | grep -q "on $MOUNT_DIR "; then IS_MOUNTED=1; fi
else
    if mountpoint -q "$MOUNT_DIR" 2>/dev/null; then IS_MOUNTED=1; fi
fi

if [[ "$IS_MOUNTED" -eq 1 ]]; then
    echo -e "${YELLOW}⚠ ${MOUNT_DIR} is already mounted — unmounting first${RESET}"
    if [[ "$OS" == "Darwin" ]]; then
        umount "$MOUNT_DIR"
    else
        fusermount3 -u "$MOUNT_DIR" 2>/dev/null || fusermount -u "$MOUNT_DIR" 2>/dev/null || umount "$MOUNT_DIR"
    fi
fi

mkdir -p "$MOUNT_DIR"
echo -e "${GREEN}✓${RESET} Mount point: ${CYAN}${MOUNT_DIR}${RESET}"
echo

# ── Build ─────────────────────────────────────────────────────────────
echo -e "${BOLD}Building secretfs (release)…${RESET}"
cargo build --release
echo -e "${GREEN}✓${RESET} Build complete: ${CYAN}${BINARY}${RESET}"
echo

# ── Mount ─────────────────────────────────────────────────────────────
echo -e "${BOLD}Mounting:${RESET}"
echo -e "  source → ${CYAN}${SOURCE_DIR}${RESET}"
echo -e "  mount  → ${CYAN}${MOUNT_DIR}${RESET}"
echo -e "  config → ${CYAN}${CONFIG}${RESET}"
echo
echo -e "${YELLOW}Tip:${RESET} In another terminal, try:"
echo -e "  ${CYAN}cat ${MOUNT_DIR}/foo.txt${RESET}          # see secrets replaced with placeholders"
echo -e "  ${CYAN}ls  ${MOUNT_DIR}/${RESET}"
echo
echo -e "${YELLOW}To unmount:${RESET}"
if [[ "$OS" == "Darwin" ]]; then
    echo -e "  ${CYAN}umount ${MOUNT_DIR}${RESET}               # or press Ctrl+C here"
else
    echo -e "  ${CYAN}fusermount3 -u ${MOUNT_DIR}${RESET}       # or press Ctrl+C here"
fi
echo

exec "$BINARY" --source "$SOURCE_DIR" --mount "$MOUNT_DIR" --config "$CONFIG"

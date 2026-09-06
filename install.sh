#!/usr/bin/env bash
# just builds buzz and drops it in /usr/local/bin, kinda like makepkg -si
# does for yay on arch
#
# run it with no args to install normally, --user if you dont wanna sudo,
# or --uninstall to remove it

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_NAME="buzz"
SYSTEM_DEST="/usr/local/bin"
USER_DEST="${HOME}/.local/bin"

INSTALL_MODE="system"
UNINSTALL=false

for arg in "$@"; do
    case "$arg" in
        --user) INSTALL_MODE="user" ;;
        --uninstall) UNINSTALL=true ;;
        -h|--help)
            echo "Usage: ./install.sh [--user] [--uninstall]"
            exit 0
            ;;
        *)
            echo "Unknown option: $arg" >&2
            exit 1
            ;;
    esac
done

log()  { echo -e "\033[32;1m[+]\033[0m $*"; }
warn() { echo -e "\033[33;1m[!]\033[0m $*"; }
die()  { echo -e "\033[31;1m[-]\033[0m $*" >&2; exit 1; }

dest_dir() {
    if [ "$INSTALL_MODE" = "user" ]; then
        echo "$USER_DEST"
    else
        echo "$SYSTEM_DEST"
    fi
}

if [ "$UNINSTALL" = true ]; then
    target="$(dest_dir)/$BIN_NAME"
    if [ -f "$target" ]; then
        if [ "$INSTALL_MODE" = "system" ]; then
            sudo rm -f "$target"
        else
            rm -f "$target"
        fi
        log "Removed $target"
    else
        warn "Nothing installed at $target"
    fi
    exit 0
fi

# 1. Make sure a Rust toolchain is available. If not, offer to install one
#    via rustup, same spirit as yay's install script checking for base-devel.
if ! command -v cargo >/dev/null 2>&1; then
    warn "cargo not found."
    read -r -p "Install Rust via rustup now? [Y/n] " reply
    reply="${reply:-Y}"
    if [[ "$reply" =~ ^[Yy] ]]; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        # shellcheck disable=SC1090
        source "${HOME}/.cargo/env"
    else
        die "cargo is required to build buzz. Install Rust and re-run this script."
    fi
fi

# 2. Build in release mode.
log "Building buzz (release) ..."
( cd "$SCRIPT_DIR" && cargo build --release )

BUILT_BIN="${SCRIPT_DIR}/target/release/${BIN_NAME}"
[ -f "$BUILT_BIN" ] || die "Build finished but ${BUILT_BIN} was not produced."

# 3. Install the binary.
DEST="$(dest_dir)"
if [ "$INSTALL_MODE" = "user" ]; then
    mkdir -p "$DEST"
    cp "$BUILT_BIN" "${DEST}/${BIN_NAME}"
    log "Installed to ${DEST}/${BIN_NAME}"
    case ":$PATH:" in
        *":${DEST}:"*) ;;
        *) warn "${DEST} is not on your PATH. Add this to your shell rc:
    export PATH=\"${DEST}:\$PATH\"" ;;
    esac
else
    sudo mkdir -p "$DEST"
    sudo cp "$BUILT_BIN" "${DEST}/${BIN_NAME}"
    sudo chmod 755 "${DEST}/${BIN_NAME}"
    log "Installed to ${DEST}/${BIN_NAME}"
fi

log "Run 'buzz --help' to get started."

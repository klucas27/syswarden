#!/usr/bin/env bash
# syswarden installer — manual install for systems without AUR access.
# Safe to re-run (idempotent). Does not start or enable the service by default.
#
# Usage:
#   ./install.sh            # build + install
#   ./install.sh --no-build # install pre-built binary (target/release/syswarden)
#   ./install.sh --enable   # also enable and start the systemd service

set -euo pipefail

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

BIN_NAME="syswarden"
BIN_SRC="target/release/${BIN_NAME}"
BIN_DEST="/usr/bin/${BIN_NAME}"
SERVICE_SRC="packaging/systemd/${BIN_NAME}.service"
SERVICE_DEST="/etc/systemd/system/${BIN_NAME}.service"
CONFIG_DIR="/etc/syswarden"
CONFIG_EXAMPLE_SRC="examples/config.balanced.toml"
CONFIG_EXAMPLE_DEST="${CONFIG_DIR}/config.toml.example"
STATE_DIRS=(
    "/var/lib/syswarden/history"
    "/var/lib/syswarden/audit"
    "/var/lib/syswarden/rollback"
)

NO_BUILD=0
ENABLE_SERVICE=0

for arg in "$@"; do
    case "$arg" in
        --no-build) NO_BUILD=1 ;;
        --enable)   ENABLE_SERVICE=1 ;;
        --help|-h)
            echo "Usage: $0 [--no-build] [--enable]"
            echo "  --no-build  Skip cargo build; use existing target/release/syswarden"
            echo "  --enable    Enable and start syswarden.service after install"
            exit 0
            ;;
        *)
            echo "Unknown option: $arg" >&2
            exit 1
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

green()  { printf '\033[32m%s\033[0m\n' "$*"; }
yellow() { printf '\033[33m%s\033[0m\n' "$*"; }
red()    { printf '\033[31m%s\033[0m\n' "$*"; }
step()   { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

die() { red "error: $*" >&2; exit 1; }

need_cmd() {
    command -v "$1" &>/dev/null || die "required command not found: $1"
}

# ---------------------------------------------------------------------------
# Preflight checks
# ---------------------------------------------------------------------------

step "Checking prerequisites"

[[ -f "Cargo.toml" ]] || die "Run this script from the syswarden source root."

need_cmd systemctl
need_cmd install

if [[ $NO_BUILD -eq 0 ]]; then
    need_cmd cargo
fi

if [[ $EUID -eq 0 ]]; then
    SUDO=""
else
    need_cmd sudo
    SUDO="sudo"
    yellow "  Will use sudo for system-level install steps."
fi

green "  Prerequisites OK."

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

if [[ $NO_BUILD -eq 0 ]]; then
    step "Building release binary"
    cargo build --release --locked
    green "  Build complete."
else
    step "Skipping build (--no-build)"
    [[ -x "$BIN_SRC" ]] || die "Binary not found at $BIN_SRC — run without --no-build first."
    yellow "  Using existing $BIN_SRC"
fi

# ---------------------------------------------------------------------------
# Install binary
# ---------------------------------------------------------------------------

step "Installing binary → ${BIN_DEST}"
$SUDO install -Dm755 "$BIN_SRC" "$BIN_DEST"
green "  Installed ${BIN_DEST}"

# ---------------------------------------------------------------------------
# Install systemd service
# ---------------------------------------------------------------------------

step "Installing systemd unit → ${SERVICE_DEST}"
$SUDO install -Dm644 "$SERVICE_SRC" "$SERVICE_DEST"
$SUDO systemctl daemon-reload
green "  Installed ${SERVICE_DEST}"

# ---------------------------------------------------------------------------
# Create state directories
# ---------------------------------------------------------------------------

step "Creating state directories"
for d in "${STATE_DIRS[@]}"; do
    $SUDO install -dm750 "$d"
    green "  ${d}"
done

# ---------------------------------------------------------------------------
# Install example config
# ---------------------------------------------------------------------------

step "Installing example config → ${CONFIG_EXAMPLE_DEST}"
$SUDO install -Dm644 "$CONFIG_EXAMPLE_SRC" "$CONFIG_EXAMPLE_DEST"
green "  ${CONFIG_EXAMPLE_DEST}"

if [[ ! -f "${CONFIG_DIR}/config.toml" ]]; then
    yellow "  No config.toml found. To create one:"
    yellow "    sudo cp ${CONFIG_EXAMPLE_DEST} ${CONFIG_DIR}/config.toml"
    yellow "    sudo \$EDITOR ${CONFIG_DIR}/config.toml"
    yellow "    syswarden config validate"
else
    green "  Existing ${CONFIG_DIR}/config.toml left untouched."
fi

# ---------------------------------------------------------------------------
# Enable / start (optional)
# ---------------------------------------------------------------------------

if [[ $ENABLE_SERVICE -eq 1 ]]; then
    step "Enabling and starting ${BIN_NAME}.service"
    $SUDO systemctl enable --now "${BIN_NAME}.service"
    green "  Service enabled and started."
    echo
    yellow "  Follow logs: journalctl -u syswarden -f"
else
    echo
    yellow "  Service not started. To start it:"
    yellow "    sudo systemctl enable --now syswarden"
    yellow "    journalctl -u syswarden -f"
fi

# ---------------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------------

echo
green "syswarden $("$BIN_DEST" version 2>/dev/null || echo "installed") — done."
echo
echo "  Config:   ${CONFIG_DIR}/config.toml  (copy from .example and edit)"
echo "  Validate: syswarden config validate"
echo "  Analyze:  syswarden analyze"
echo "  Dry-run:  syswarden actions dry-run"
echo "  Uninstall: ./uninstall.sh"

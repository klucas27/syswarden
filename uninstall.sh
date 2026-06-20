#!/usr/bin/env bash
# syswarden uninstaller.
# Stops and removes the service, binary, and unit file.
# Does NOT delete /var/lib/syswarden/ (history, audit, rollback) by default.
#
# Usage:
#   ./uninstall.sh            # remove binary + service, keep state data
#   ./uninstall.sh --purge    # also delete /var/lib/syswarden/ and /etc/syswarden/

set -euo pipefail

BIN_DEST="/usr/bin/syswarden"
SERVICE_DEST="/etc/systemd/system/syswarden.service"
CONFIG_DIR="/etc/syswarden"
STATE_DIR="/var/lib/syswarden"

PURGE=0
for arg in "$@"; do
    case "$arg" in
        --purge) PURGE=1 ;;
        --help|-h)
            echo "Usage: $0 [--purge]"
            echo "  --purge  Also delete /var/lib/syswarden/ and /etc/syswarden/"
            exit 0
            ;;
        *)
            echo "Unknown option: $arg" >&2
            exit 1
            ;;
    esac
done

green()  { printf '\033[32m%s\033[0m\n' "$*"; }
yellow() { printf '\033[33m%s\033[0m\n' "$*"; }
step()   { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

if [[ $EUID -eq 0 ]]; then
    SUDO=""
else
    command -v sudo &>/dev/null || { echo "error: sudo required" >&2; exit 1; }
    SUDO="sudo"
fi

step "Stopping and disabling syswarden.service"
if systemctl is-active --quiet syswarden 2>/dev/null; then
    $SUDO systemctl stop syswarden
    green "  Stopped."
else
    yellow "  Service not running."
fi
if systemctl is-enabled --quiet syswarden 2>/dev/null; then
    $SUDO systemctl disable syswarden
    green "  Disabled."
else
    yellow "  Service not enabled."
fi

step "Removing systemd unit → ${SERVICE_DEST}"
if [[ -f "$SERVICE_DEST" ]]; then
    $SUDO rm -f "$SERVICE_DEST"
    $SUDO systemctl daemon-reload
    green "  Removed."
else
    yellow "  Not found (already removed?)."
fi

step "Removing binary → ${BIN_DEST}"
if [[ -f "$BIN_DEST" ]]; then
    $SUDO rm -f "$BIN_DEST"
    green "  Removed."
else
    yellow "  Not found (already removed?)."
fi

if [[ $PURGE -eq 1 ]]; then
    step "Purging state data → ${STATE_DIR}"
    if [[ -d "$STATE_DIR" ]]; then
        $SUDO rm -rf "$STATE_DIR"
        green "  Deleted ${STATE_DIR}/"
    else
        yellow "  Not found."
    fi

    step "Purging config → ${CONFIG_DIR}"
    if [[ -d "$CONFIG_DIR" ]]; then
        $SUDO rm -rf "$CONFIG_DIR"
        green "  Deleted ${CONFIG_DIR}/"
    else
        yellow "  Not found."
    fi
else
    echo
    yellow "  State data preserved at ${STATE_DIR}/ (history, audit, rollback)."
    yellow "  Run with --purge to also delete it."
fi

echo
green "syswarden uninstalled."

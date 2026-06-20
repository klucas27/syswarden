#!/usr/bin/env bash
# Shared bootstrap — sourced by profile activation scripts, not run directly.
#
# Provides: check_root, ensure_rust, ensure_syswarden, restart_or_enable,
#           first_cleanup, profile_summary
# Sets:     REPO_ROOT, CONFIG_DIR, CONFIG_FILE

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CONFIG_DIR="/etc/syswarden"
CONFIG_FILE="$CONFIG_DIR/config.toml"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

green()  { printf '\033[32m%s\033[0m\n' "$*"; }
yellow() { printf '\033[33m%s\033[0m\n' "$*"; }
red()    { printf '\033[31m%s\033[0m\n' "$*"; }
step()   { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
die()    { red "error: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Root check
# ---------------------------------------------------------------------------

check_root() {
    [[ $EUID -eq 0 ]] || die "must run as root — prefix with: sudo"
}

# ---------------------------------------------------------------------------
# Rust / cargo
# ---------------------------------------------------------------------------

_cargo_in_path() {
    # When invoked via `sudo`, the invoking user's ~/.cargo/bin is not in PATH.
    # Try both the original user's home and root's home before giving up.
    if [[ -n "${SUDO_USER:-}" ]]; then
        local uhome
        uhome="$(getent passwd "$SUDO_USER" 2>/dev/null | cut -d: -f6)" || uhome=""
        if [[ -n "$uhome" ]]; then
            export PATH="${uhome}/.cargo/bin:$PATH"
            # shellcheck source=/dev/null
            [[ -f "${uhome}/.cargo/env" ]] && source "${uhome}/.cargo/env" 2>/dev/null || true
        fi
    fi
    # Also try root's installation (in case rustup was run as root previously)
    # shellcheck source=/dev/null
    [[ -f /root/.cargo/env ]] && source /root/.cargo/env 2>/dev/null || true
    command -v cargo &>/dev/null
}

ensure_rust() {
    if _cargo_in_path; then
        green "  cargo: $(cargo --version 2>&1 | head -1)"
        return 0
    fi

    step "cargo not found — installing Rust via rustup"
    if ! command -v curl &>/dev/null && ! command -v wget &>/dev/null; then
        die "curl or wget is required to install Rust — install one and retry"
    fi

    local tmp_installer
    tmp_installer="$(mktemp /tmp/rustup-init.XXXXXX.sh)"
    # shellcheck disable=SC2064
    trap "rm -f '$tmp_installer'" EXIT

    if command -v curl &>/dev/null; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$tmp_installer"
    else
        wget --quiet -O "$tmp_installer" https://sh.rustup.rs
    fi
    chmod +x "$tmp_installer"

    if [[ -n "${SUDO_USER:-}" ]]; then
        # Install under the invoking user — cargo registry lives in their home.
        local uhome
        uhome="$(getent passwd "$SUDO_USER" | cut -d: -f6)"
        sudo -u "$SUDO_USER" "$tmp_installer" -y --no-modify-path
        export PATH="${uhome}/.cargo/bin:$PATH"
        # shellcheck source=/dev/null
        source "${uhome}/.cargo/env" 2>/dev/null || true
    else
        "$tmp_installer" -y --no-modify-path
        # shellcheck source=/dev/null
        source "$HOME/.cargo/env" 2>/dev/null || true
    fi

    command -v cargo &>/dev/null \
        || die "Rust install failed — add ~/.cargo/bin to PATH and retry"
    green "  Rust installed: $(cargo --version)"
}

# ---------------------------------------------------------------------------
# Build + install syswarden
# ---------------------------------------------------------------------------

_build() {
    step "Building syswarden (release)"
    [[ -f "$REPO_ROOT/Cargo.toml" ]] \
        || die "Cargo.toml not found at $REPO_ROOT — run this script from the syswarden repo"

    if [[ -n "${SUDO_USER:-}" ]]; then
        # Build as the original user so cargo's registry cache stays in their home.
        sudo -u "$SUDO_USER" bash -c "cd '$REPO_ROOT' && cargo build --release --locked"
    else
        (cd "$REPO_ROOT" && cargo build --release --locked)
    fi

    [[ -x "$REPO_ROOT/target/release/syswarden" ]] \
        || die "Build failed — no binary at $REPO_ROOT/target/release/syswarden"
    green "  Build OK"
}

_install_files() {
    step "Installing syswarden"
    install -Dm755 "$REPO_ROOT/target/release/syswarden" /usr/bin/syswarden
    install -Dm644 "$REPO_ROOT/packaging/systemd/syswarden.service" \
                   /etc/systemd/system/syswarden.service
    install -dm750 /var/lib/syswarden/history
    install -dm750 /var/lib/syswarden/audit
    install -dm750 /var/lib/syswarden/rollback
    mkdir -p "$CONFIG_DIR"
    [[ -f "$REPO_ROOT/examples/config.balanced.toml" ]] && \
        install -Dm644 "$REPO_ROOT/examples/config.balanced.toml" \
                       "$CONFIG_DIR/config.toml.example" 2>/dev/null || true
    systemctl daemon-reload
    green "  /usr/bin/syswarden"
    green "  /etc/systemd/system/syswarden.service"
    green "  /var/lib/syswarden/{history,audit,rollback}"
}

ensure_syswarden() {
    if [[ -x /usr/bin/syswarden ]]; then
        local ver
        ver="$(/usr/bin/syswarden version 2>/dev/null || echo 'already installed')"
        green "  syswarden: $ver"
        return 0
    fi

    ensure_rust
    _build
    _install_files
}

# ---------------------------------------------------------------------------
# Service control
# ---------------------------------------------------------------------------

restart_or_enable() {
    local profile="$1"
    step "Activating profile: $profile"
    # `enable --now` makes syswarden start now AND on every boot.
    if systemctl is-enabled --quiet syswarden.service 2>/dev/null; then
        systemctl restart syswarden.service
        green "  syswarden restarted (already enabled at boot)"
    else
        systemctl enable --now syswarden.service
        green "  syswarden enabled at boot and started"
    fi
}

profile_summary() {
    local profile="$1"
    echo
    yellow "  Profile : $profile"
    yellow "  Config  : $CONFIG_FILE"
    yellow "  Logs    : journalctl -u syswarden -f"
    yellow ""
    yellow "  Next: edit $CONFIG_FILE → [allowed].services"
    yellow "  to list the services syswarden should govern."
}

# ---------------------------------------------------------------------------
# First cleanup — owner-approved activation-time reclaim (architecture.md §17.1)
#
# This is SCRIPT-LEVEL, one-time, and runs OUTSIDE the daemon's safety gate.
# It is destructive (drops caches, SIGTERMs heavy user processes), so it is
# gated by an interactive confirmation and is safe-by-construction: it can
# never take the system (or itself) down. It NEVER touches:
#   - system/root processes (uid < 1000)        -> all daemons survive
#   - kernel threads (no /proc/<pid>/exe)
#   - syswarden, sshd, this script's own tree    -> never-kill set
#   - the invoking user's login/desktop session  -> desktop survives
# Processes are signalled with SIGTERM only (graceful) — never SIGKILL.
# Skipped entirely for the conservative profile (which promises zero touch).
# ---------------------------------------------------------------------------

# Print the invoking user's login-session leader PIDs and all their descendants.
_session_tree_pids() {
    [[ -n "${SUDO_USER:-}" ]] || return 0
    command -v loginctl &>/dev/null || return 0
    local sessions s leader
    sessions="$(loginctl list-sessions --no-legend 2>/dev/null \
        | awk -v u="$SUDO_USER" '$3 == u { print $1 }')" || return 0
    for s in $sessions; do
        leader="$(loginctl show-session "$s" -p Leader --value 2>/dev/null)" || continue
        [[ "$leader" =~ ^[0-9]+$ ]] || continue
        _descendants "$leader"
    done
}

# Recursively print a pid and all of its descendants.
_descendants() {
    local root="$1" k
    echo "$root"
    for k in $(pgrep -P "$root" 2>/dev/null); do
        _descendants "$k"
    done
}

# Comms that are session/desktop-critical and must never be signalled, even if
# they somehow escape the session-tree exclusion. Defense in depth.
_CRITICAL_COMMS="Xorg Xwayland gnome-shell plasmashell kwin_wayland kwin_x11 kwin \
sway mutter gdm gdm-session-wor sddm sddm-helper lightdm ksmserver gnome-session-b \
pipewire pipewire-pulse wireplumber pulseaudio dbus-daemon dbus-broker systemd \
gnome-keyring-d polkitd"

# PIDs that must survive no matter what: self + ancestors, syswarden, sshd, the
# invoking user's login-session tree, AND their whole `systemd --user` session
# (plasmashell/pipewire/etc. are children of `systemd --user`, not of loginctl).
_never_kill_set() {
    local p=$$
    while [[ "${p:-0}" -gt 1 ]]; do
        echo "$p"
        p="$(ps -o ppid= -p "$p" 2>/dev/null | tr -d ' ')"
        [[ -n "$p" ]] || break
    done
    pgrep -x syswarden 2>/dev/null || true
    pgrep -x sshd 2>/dev/null || true
    _session_tree_pids
    if [[ -n "${SUDO_USER:-}" ]]; then
        local m
        for m in $(pgrep -u "$SUDO_USER" -x systemd 2>/dev/null); do
            _descendants "$m"
        done
    fi
    echo 1
}

# Print "pid rss_kb comm" for heavy, killable user processes only.
_heavy_user_pids() {
    local never="$1"
    local thresh_kb=$(( ${SYSWARDEN_CLEAN_RSS_MB:-300} * 1024 ))
    ps -eo uid=,pid=,rss=,comm= 2>/dev/null | while read -r uid pid rss comm; do
        [[ "$uid" =~ ^[0-9]+$ ]] || continue
        [[ "$uid" -ge 1000 ]] || continue          # system/root untouched
        [[ "$rss" -ge "$thresh_kb" ]] || continue  # only heavy
        [[ -e "/proc/$pid/exe" ]] || continue      # skip kernel threads
        grep -qx "$pid" <<< "$never" && continue   # never-kill set
        [[ " $_CRITICAL_COMMS " == *" $comm "* ]] && continue  # session-critical
        printf '%s %s %s\n' "$pid" "$rss" "$comm"
    done
}

first_cleanup() {
    local profile="$1"
    # Conservative promises zero system touch — never clean.
    if [[ "$profile" == "conservative" ]]; then
        return 0
    fi

    step "First cleanup (memory + processes)"
    red    "  WARNING — this is destructive and runs OUTSIDE syswarden's safety gate:"
    red    "    • sync + drop page/dentry/inode caches (/proc/sys/vm/drop_caches)"
    red    "    • SIGTERM heavy NON-system, NON-desktop user processes (graceful)"
    yellow "  Protected: kernel threads, system procs (uid<1000), syswarden, sshd,"
    yellow "  and your login/desktop session are NEVER touched — the system stays up."

    # Confirmation gate. Non-interactive shells skip unless SYSWARDEN_ASSUME_YES=1.
    if [[ "${SYSWARDEN_ASSUME_YES:-0}" != "1" ]]; then
        if [[ ! -t 0 ]]; then
            yellow "  Non-interactive shell and SYSWARDEN_ASSUME_YES!=1 — skipping cleanup."
            return 0
        fi
        local ans=""
        read -r -p "  Proceed with first cleanup? [y/N] " ans || ans=""
        if [[ "${ans,,}" != "y" && "${ans,,}" != "yes" ]]; then
            yellow "  Cleanup skipped."
            return 0
        fi
    fi

    # --- memory reclaim (safe) ---
    sync
    if echo 3 > /proc/sys/vm/drop_caches 2>/dev/null; then
        green "  caches dropped"
    else
        yellow "  could not drop caches — skipped"
    fi
    if [[ -w /proc/sys/vm/compact_memory ]] && echo 1 > /proc/sys/vm/compact_memory 2>/dev/null; then
        green "  memory compacted"
    fi

    # --- process reclaim (gated, graceful) ---
    local never targets
    never="$(_never_kill_set | sort -un)"
    targets="$(_heavy_user_pids "$never")"
    if [[ -z "$targets" ]]; then
        green "  no heavy user processes to clean (>= ${SYSWARDEN_CLEAN_RSS_MB:-300} MB)"
        return 0
    fi

    yellow "  Candidate processes (graceful SIGTERM):"
    printf '%s\n' "$targets" | while read -r pid rss comm; do
        printf '    pid %-7s %6s MB  %s\n' "$pid" "$(( rss / 1024 ))" "$comm"
    done

    if [[ "${SYSWARDEN_ASSUME_YES:-0}" != "1" ]]; then
        local ans2=""
        read -r -p "  Terminate the listed processes? [y/N] " ans2 || ans2=""
        if [[ "${ans2,,}" != "y" && "${ans2,,}" != "yes" ]]; then
            yellow "  Process cleanup skipped."
            return 0
        fi
    fi

    local pid rss comm
    printf '%s\n' "$targets" | while read -r pid rss comm; do
        if kill -TERM "$pid" 2>/dev/null; then
            green "  SIGTERM -> pid $pid ($comm)"
        fi
    done
}

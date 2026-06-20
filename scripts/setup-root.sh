#!/usr/bin/env bash
# setup-root.sh — Configure syswarden with performance profile and start as root.
#
# Services added to allowed.services (real systemd daemons):
#   ollama.service, docker.service, containerd.service, vboxweb.service
#
# Apps controlled as PROCESSES (nice/ionice — no .service needed):
#   chromium, code (VSCode), cursor, antigravity-ide, idea (IntelliJ),
#   VBoxHeadless (VirtualBox VMs)
#
# Usage: sudo bash scripts/setup-root.sh

set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
ok()   { echo -e "${GREEN}✓${NC} $*"; }
warn() { echo -e "${YELLOW}⚠${NC} $*"; }
info() { echo -e "${CYAN}→${NC} $*"; }
err()  { echo -e "${RED}✗${NC} $*"; }

# ---------------------------------------------------------------------------
# Guard: root required
# ---------------------------------------------------------------------------
if [[ $EUID -ne 0 ]]; then
    err "Este script precisa de root: sudo bash $0"
    exit 1
fi

CONFIG=/etc/syswarden/config.toml

echo ""
echo "========================================"
echo "  syswarden — setup performance + apps"
echo "========================================"
echo ""

# ---------------------------------------------------------------------------
# 1. Backup config
# ---------------------------------------------------------------------------
BACKUP="${CONFIG}.bak.$(date +%Y%m%d_%H%M%S)"
cp "$CONFIG" "$BACKUP"
ok "Config salvo em $BACKUP"

# ---------------------------------------------------------------------------
# 2. Verify which services actually exist on this system
# ---------------------------------------------------------------------------
info "Verificando serviços disponíveis no sistema..."

SERVICES_TO_ALLOW=()

check_service() {
    local svc="$1"
    if systemctl list-unit-files "$svc" 2>/dev/null | grep -q "$svc"; then
        SERVICES_TO_ALLOW+=("$svc")
        ok "  $svc encontrado"
    else
        warn "  $svc não encontrado (ignorado)"
    fi
}

check_service "ollama.service"
check_service "docker.service"
check_service "containerd.service"
check_service "vboxweb.service"

echo ""
info "Apps GUI (controlados como PROCESSOS via nice/ionice — não precisam de .service):"
for app in chromium code cursor antigravity-ide idea VBoxHeadless; do
    if command -v "$app" &>/dev/null; then
        ok "  $app → $(which $app)"
    else
        warn "  $app não encontrado no PATH"
    fi
done

# ---------------------------------------------------------------------------
# 3. Update config with Python (safe TOML edit)
# ---------------------------------------------------------------------------
echo ""
info "Atualizando /etc/syswarden/config.toml..."

python3 - "$CONFIG" "${SERVICES_TO_ALLOW[@]}" << 'PYEOF'
import sys, re

config_path   = sys.argv[1]
new_allowed   = sys.argv[2:]

with open(config_path) as f:
    lines = f.readlines()

# Line-by-line state machine: replace services = [...] ONLY in [allowed].
# Never touches [protected] or any other section.
section = None
result  = []
i = 0
while i < len(lines):
    line = lines[i]
    m = re.match(r'^\[([^\]]+)\]', line)
    if m:
        section = m.group(1)

    if section == 'allowed' and re.match(r'\s*services\s*=\s*\[', line):
        result.append('services = [\n')
        for s in new_allowed:
            result.append(f'    "{s}",\n')
        # skip the original block until closing ]
        i += 1
        while i < len(lines) and lines[i].strip() != ']':
            i += 1
        result.append(']\n')
        i += 1
        continue

    result.append(line)
    i += 1

content = ''.join(result)
# Force profile and dry_run
content = re.sub(r'(profile\s*=\s*)"[^"]*"', r'\1"performance"', content)
content = re.sub(r'(dry_run\s*=\s*)(true|false)', r'\1false', content)

with open(config_path, 'w') as f:
    f.write(content)

print(f"Updated: profile=performance, dry_run=false, {len(new_allowed)} service(s) in [allowed]")
PYEOF

ok "Config atualizado"

# ---------------------------------------------------------------------------
# 4. Show what changed
# ---------------------------------------------------------------------------
echo ""
info "Config atual [global] e [allowed]:"
grep -A3 '^\[global\]' "$CONFIG" | head -6
echo "..."
grep -A10 '^\[allowed\]' "$CONFIG"

# ---------------------------------------------------------------------------
# 5. Validate config
# ---------------------------------------------------------------------------
echo ""
info "Validando config..."
if syswarden config validate; then
    ok "Config válido"
else
    err "Problemas na config — verifique acima"
    exit 1
fi

# ---------------------------------------------------------------------------
# 6. System capabilities
# ---------------------------------------------------------------------------
echo ""
info "Capacidades do sistema:"
syswarden doctor

# ---------------------------------------------------------------------------
# 7. Stop any existing daemon
# ---------------------------------------------------------------------------
echo ""
info "Parando daemon anterior (se houver)..."
if systemctl is-active --quiet syswarden 2>/dev/null; then
    systemctl stop syswarden
    ok "Serviço systemd parado"
fi
# Kill any manual daemon
pkill -f "syswarden daemon" 2>/dev/null && warn "Processo anterior encerrado" || true

# ---------------------------------------------------------------------------
# 8. Start daemon as root
# ---------------------------------------------------------------------------
echo ""
info "Iniciando syswarden daemon..."
LOG=/var/log/syswarden.log
nohup syswarden daemon > "$LOG" 2>&1 &
SWPID=$!
echo "$SWPID" > /var/run/syswarden.pid

sleep 2

if kill -0 "$SWPID" 2>/dev/null; then
    ok "Daemon rodando — PID $SWPID"
    ok "Log: $LOG"
else
    err "Daemon falhou ao iniciar — veja $LOG"
    tail -20 "$LOG"
    exit 1
fi

# ---------------------------------------------------------------------------
# 9. Show initial dry-run to confirm it can see pressure
# ---------------------------------------------------------------------------
echo ""
info "Estado atual (o que faria agora):"
syswarden actions dry-run

# ---------------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------------
echo ""
echo "========================================"
echo -e "${GREEN}  Setup completo!${NC}"
echo "========================================"
echo ""
echo "Comandos úteis:"
echo "  tail -f $LOG              # ver logs em tempo real"
echo "  syswarden rollback list   # ver ações aplicadas"
echo "  sudo bash scripts/test-live.sh  # rodar teste real"
echo "  kill \$(cat /var/run/syswarden.pid)  # parar daemon"
echo ""

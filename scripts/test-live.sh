#!/usr/bin/env bash
# test-live.sh — Teste real de syswarden: gera pressão de CPU e memória.
#
# Fases:
#   1. CPU stress  (30s) — workers Python fazem factorial em loop
#   2. Mem stress  (30s) — worker Python aloca ≤50% da RAM disponível e toca cada página
#
# Para cada fase monitora PSI (cpu + mem), nice do worker líder, e
# novas entradas de rollback do syswarden.
#
# Usage: sudo bash scripts/test-live.sh

set -euo pipefail

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'
ok()   { echo -e "${GREEN}✓${NC} $*"; }
warn() { echo -e "${YELLOW}⚠${NC} $*"; }
info() { echo -e "${CYAN}→${NC} $*"; }
err()  { echo -e "${RED}✗${NC} $*"; }
hdr()  { echo -e "\n${BOLD}$*${NC}"; }

# ---------------------------------------------------------------------------
# Guard: root required
# ---------------------------------------------------------------------------
if [[ $EUID -ne 0 ]]; then
    err "Precisa de root: sudo bash $0"
    exit 1
fi

# ---------------------------------------------------------------------------
# 1. Check daemon is running
# ---------------------------------------------------------------------------
hdr "=== Verificando daemon ==="

SW_PID=""
if [[ -f /var/run/syswarden.pid ]]; then
    SW_PID=$(cat /var/run/syswarden.pid)
fi

if [[ -n "$SW_PID" ]] && kill -0 "$SW_PID" 2>/dev/null; then
    ok "Daemon rodando — PID $SW_PID"
elif pgrep -f "syswarden daemon" &>/dev/null; then
    SW_PID=$(pgrep -f "syswarden daemon" | head -1)
    ok "Daemon rodando — PID $SW_PID"
else
    err "Daemon NÃO está rodando!"
    echo "  Inicie com: sudo bash scripts/setup-root.sh"
    echo "  Ou: sudo syswarden daemon &"
    exit 1
fi

# ---------------------------------------------------------------------------
# 2. Stress generators (pure Python — sem dependências externas)
# ---------------------------------------------------------------------------
USE_STRESS_NG=false
if command -v stress-ng &>/dev/null; then
    USE_STRESS_NG=true
    ok "stress-ng disponível"
else
    warn "stress-ng não encontrado — usando geradores Python puros"
fi

CPU_PIDS=()
MEM_PIDS=()

start_cpu_stress() {
    local ncpu="$1" duration="$2"
    CPU_PIDS=()
    if $USE_STRESS_NG; then
        stress-ng --cpu "$ncpu" --cpu-method fft --timeout "${duration}s" --quiet &
        CPU_PIDS=($!)
    else
        local py='
import math, time, sys
end = time.time() + int(sys.argv[1])
while time.time() < end:
    _ = math.factorial(10000)
'
        for (( c=0; c<ncpu; c++ )); do
            python3 -c "$py" "$duration" &
            CPU_PIDS+=($!)
        done
        ok "CPU: $ncpu workers Python (PIDs: ${CPU_PIDS[*]})"
    fi
}

start_mem_stress() {
    local duration="$1"
    MEM_PIDS=()
    if $USE_STRESS_NG; then
        # Allocate 50% of available RAM
        local avail_mb
        avail_mb=$(awk '/MemAvailable:/{print int($2/1024)}' /proc/meminfo)
        local target_mb=$(( avail_mb * 50 / 100 ))
        stress-ng --vm 1 --vm-bytes "${target_mb}M" --vm-method all \
                  --timeout "${duration}s" --quiet &
        MEM_PIDS=($!)
        ok "Mem: stress-ng vm ${target_mb}MB por ${duration}s"
    else
        # Pure Python: allocate ≤50% of MemAvailable, touch every page
        local py='
import time, sys

duration = int(sys.argv[1])

with open("/proc/meminfo") as f:
    for line in f:
        if line.startswith("MemAvailable:"):
            avail_kb = int(line.split()[1])
            break

target_bytes = avail_kb * 1024 * 50 // 100
chunk_size   = 50 * 1024 * 1024  # 50 MB per chunk

chunks = []
allocated = 0
while allocated < target_bytes:
    size = min(chunk_size, target_bytes - allocated)
    buf  = bytearray(size)
    # Touch every OS page (4 KB) to force physical allocation
    for i in range(0, size, 4096):
        buf[i] = 1
    chunks.append(buf)
    allocated += size

# Hold until duration expires
time.sleep(max(0, duration - (time.time() % 1)))
'
        python3 -c "$py" "$duration" &
        MEM_PIDS+=($!)
        local avail_mb
        avail_mb=$(awk '/MemAvailable:/{print int($2/1024)}' /proc/meminfo)
        ok "Mem: Python alocando ≤$((avail_mb * 50 / 100))MB por ${duration}s (PID: ${MEM_PIDS[*]})"
    fi
}

stop_pids() {
    for pid in "$@"; do kill "$pid" 2>/dev/null || true; done
}

# ---------------------------------------------------------------------------
# 3. Snapshot inicial
# ---------------------------------------------------------------------------
hdr "=== Estado ANTES do teste ==="

syswarden doctor | grep -E "root:|PSI:|cgroup|profile:|dry_run:"
echo ""

BEFORE_LIST=$(syswarden rollback list 2>&1 || echo "")
BEFORE_COUNT=$(echo "$BEFORE_LIST" | grep -cE "AdjustNice|AdjustIonice|SetCpuWeight|SetIoWeight|SetMemoryHigh" 2>/dev/null || true)
BEFORE_COUNT=${BEFORE_COUNT:-0}
echo "Rollback entries existentes: $BEFORE_COUNT"
[[ $BEFORE_COUNT -gt 0 ]] && echo "$BEFORE_LIST" | tail -5 || true

CPU_PSI_BEFORE=$(awk '/^some/ {print $2}' /proc/pressure/cpu    2>/dev/null | cut -d= -f2 || echo "N/A")
MEM_PSI_BEFORE=$(awk '/^some/ {print $2}' /proc/pressure/memory 2>/dev/null | cut -d= -f2 || echo "N/A")
info "CPU PSI avg10: ${CPU_PSI_BEFORE}%  |  MEM PSI avg10: ${MEM_PSI_BEFORE}%"

# ---------------------------------------------------------------------------
# Monitoramento genérico
# ---------------------------------------------------------------------------
NCPU=$(nproc)
MAX_CPU_PSI=0
MAX_MEM_PSI=0
ACTIONS_SEEN=0

monitor_loop() {
    local duration="$1" leader_pid="$2" phase="$3"
    local i=0

    hdr "=== Monitorando fase: $phase ==="
    printf "%-6s %-12s %-12s %-10s %-6s %s\n" \
        "Seg" "CPU-PSI10" "MEM-PSI10" "LeaderNice" "SW-act" "Veredicto"
    echo "--------------------------------------------------------------------------"

    for i in $(seq 1 $((duration + 8))); do
        sleep 1

        local cpu_psi mem_psi
        cpu_psi=$(awk '/^some/ {print $2}' /proc/pressure/cpu    2>/dev/null | cut -d= -f2 || echo "0")
        mem_psi=$(awk '/^some/ {print $2}' /proc/pressure/memory 2>/dev/null | cut -d= -f2 || echo "0")

        local nice="n/a"
        if kill -0 "$leader_pid" 2>/dev/null; then
            nice=$(awk '/^Nice:/{print $2}' /proc/"$leader_pid"/status 2>/dev/null || echo "?")
        else
            nice="done"
        fi

        local curr
        curr=$(syswarden rollback list 2>/dev/null \
            | grep -cE "AdjustNice|AdjustIonice|SetCpuWeight|SetIoWeight|SetMemoryHigh" 2>/dev/null || true)
        curr=${curr:-0}
        local new_actions=$(( curr - BEFORE_COUNT ))

        # Track maxima
        if (( $(echo "$cpu_psi > $MAX_CPU_PSI" | bc -l 2>/dev/null || echo 0) )); then MAX_CPU_PSI=$cpu_psi; fi
        if (( $(echo "$mem_psi > $MAX_MEM_PSI" | bc -l 2>/dev/null || echo 0) )); then MAX_MEM_PSI=$mem_psi; fi

        local verdict="PSI baixo"
        if [[ "$nice" == "done" ]]; then
            verdict="fase concluída"
        elif [[ $new_actions -gt $ACTIONS_SEEN ]]; then
            verdict=">>> AÇÃO APLICADA!"
            ACTIONS_SEEN=$new_actions
        elif (( $(echo "$cpu_psi > 15" | bc -l 2>/dev/null || echo 0) )) || \
             (( $(echo "$mem_psi > 10" | bc -l 2>/dev/null || echo 0) )); then
            verdict="aguardando daemon..."
        fi

        printf "%-6s %-12s %-12s %-10s %-6s %s\n" \
            "[${i}s]" "${cpu_psi}%" "${mem_psi}%" "$nice" "$new_actions" "$verdict"

        [[ "$nice" == "done" ]] && sleep 2 && break
    done
}

# ---------------------------------------------------------------------------
# 4. FASE 1 — CPU stress
# ---------------------------------------------------------------------------
CPU_DURATION=30
hdr "=== FASE 1: Estresse de CPU ($CPU_DURATION s) ==="
info "Lançando $NCPU workers..."
echo ""

start_cpu_stress "$NCPU" "$CPU_DURATION"
CPU_LEADER="${CPU_PIDS[0]}"

monitor_loop "$CPU_DURATION" "$CPU_LEADER" "CPU"

stop_pids "${CPU_PIDS[@]}"
for pid in "${CPU_PIDS[@]}"; do wait "$pid" 2>/dev/null || true; done

# ---------------------------------------------------------------------------
# 5. FASE 2 — Memória stress
# ---------------------------------------------------------------------------
MEM_DURATION=30
hdr "=== FASE 2: Estresse de Memória ($MEM_DURATION s) ==="
info "Alocando memória (≤50% da RAM disponível)..."
echo ""

start_mem_stress "$MEM_DURATION"
MEM_LEADER="${MEM_PIDS[0]}"

monitor_loop "$MEM_DURATION" "$MEM_LEADER" "Memória"

stop_pids "${MEM_PIDS[@]}"
for pid in "${MEM_PIDS[@]}"; do wait "$pid" 2>/dev/null || true; done

# ---------------------------------------------------------------------------
# 6. Relatório final
# ---------------------------------------------------------------------------
hdr "=== Resultado Final ==="

AFTER_LIST=$(syswarden rollback list 2>&1 || echo "")
AFTER_COUNT=$(echo "$AFTER_LIST" | grep -cE "AdjustNice|AdjustIonice|SetCpuWeight|SetIoWeight|SetMemoryHigh" 2>/dev/null || true)
AFTER_COUNT=${AFTER_COUNT:-0}
TOTAL_NEW=$(( AFTER_COUNT - BEFORE_COUNT ))

echo ""
echo "Ações aplicadas durante o teste: $TOTAL_NEW"
echo "PSI máximo CPU:                  ${MAX_CPU_PSI}%  (threshold: 15%)"
echo "PSI máximo Memória:              ${MAX_MEM_PSI}%  (threshold: 10%)"
echo ""

if [[ $TOTAL_NEW -gt 0 ]]; then
    echo -e "${GREEN}╔════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║  ✓  syswarden FUNCIONANDO CORRETAMENTE ║${NC}"
    echo -e "${GREEN}║     $TOTAL_NEW ação(ões) real(is) aplicada(s)    ║${NC}"
    echo -e "${GREEN}╚════════════════════════════════════════╝${NC}"
    echo ""
    echo "Últimas entradas de rollback:"
    echo "$AFTER_LIST" | tail -10
    echo ""
    LAST_ID=$(echo "$AFTER_LIST" | grep -E "AdjustNice|AdjustIonice|SetCpu" | tail -1 \
        | grep -oE 'id=[0-9]+' | cut -d= -f2 || echo "")
    [[ -n "$LAST_ID" ]] && echo "Para reverter: sudo syswarden rollback apply $LAST_ID"
else
    echo -e "${YELLOW}╔══════════════════════════════════════════════╗${NC}"
    echo -e "${YELLOW}║  ⚠  Nenhuma ação nova detectada              ║${NC}"
    echo -e "${YELLOW}╚══════════════════════════════════════════════╝${NC}"
    echo ""
    echo "Possíveis causas:"

    if (( $(echo "$MAX_CPU_PSI < 15" | bc -l 2>/dev/null || echo 1) )) && \
       (( $(echo "$MAX_MEM_PSI < 10" | bc -l 2>/dev/null || echo 1) )); then
        warn "PSI ficou abaixo dos thresholds (CPU ${MAX_CPU_PSI}% / Mem ${MAX_MEM_PSI}%)"
        echo "     → Tenta fechar outros apps para ter mais RAM disponível"
        echo "     → Ou reduz os thresholds em /etc/syswarden/config.toml temporariamente"
    fi

    if ! kill -0 "$SW_PID" 2>/dev/null; then
        warn "Daemon parou durante o teste — veja /var/log/syswarden.log"
    fi

    echo ""
    echo "Log do daemon (últimas 30 linhas):"
    tail -30 /var/log/syswarden.log 2>/dev/null || echo "(log não encontrado)"
fi

echo ""
info "Log completo:    tail -f /var/log/syswarden.log"
info "Rollback list:   syswarden rollback list"

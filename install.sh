#!/usr/bin/env bash
# ──────────────────────────────────────────────
# Nyquest v3.2.0 Installer — From zero to running
# One-shot: system deps, Rust, build, preflight, semantic stage, systemd
# Usage: curl -sSfL https://raw.githubusercontent.com/Nyquest-ai/nyquest-rust-fullstack-3.2.0/main/install.sh | bash
# ──────────────────────────────────────────────
set -uo pipefail
# NOTE: We intentionally do NOT use 'set -e' because third-party
# apt repos, GPU drivers, and other system state can cause non-fatal
# errors that should not kill the installer.

REPO="https://github.com/Nyquest-ai/nyquest-rust-fullstack-3.2.0.git"
BRANCH="main"
INSTALL_DIR="$HOME/nyquest"
SEMANTIC_MODEL="qwen2.5:1.5b-instruct"
OLLAMA_PORT=11434
BOLD='\033[1m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
DIM='\033[2m'
NC='\033[0m'

info()  { echo -e "${CYAN}[nyquest]${NC} $1"; }
ok()    { echo -e "${GREEN}[  ✓  ]${NC} $1"; }
warn()  { echo -e "${YELLOW}[ warn]${NC} $1"; }
fail()  { echo -e "${RED}[error]${NC} $1"; exit 1; }
step()  { echo -e "\n${BOLD}${CYAN}── $1 ──${NC}"; }

echo -e "${BOLD}${CYAN}"
cat << 'BANNER'
   ╔═══════════════════════════════════════════════════════╗
   ║  ⚡ NYQUEST v3.2.0 — ONE-SHOT INSTALLER              ║
   ║  Semantic Compression Proxy for LLMs                 ║
   ║  Full Rust Engine + Local LLM Semantic Stage         ║
   ╚═══════════════════════════════════════════════════════╝
BANNER
echo -e "${NC}"

# ═══════════════════════════════════════════════════════
# PHASE 1: Hardware Pre-Check
# ═══════════════════════════════════════════════════════
step "Phase 1: Hardware Pre-Check"

# OS
OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS" in
    Linux)  PLATFORM="linux" ;;
    Darwin) PLATFORM="macos" ;;
    *)      fail "Unsupported OS: $OS" ;;
esac
ok "Platform: $PLATFORM ($ARCH)"

# CPU cores
if [ "$PLATFORM" = "linux" ]; then
    CPU_CORES=$(nproc 2>/dev/null || grep -c ^processor /proc/cpuinfo 2>/dev/null || echo "?")
else
    CPU_CORES=$(sysctl -n hw.ncpu 2>/dev/null || echo "?")
fi
if [ "$CPU_CORES" != "?" ] && [ "$CPU_CORES" -ge 2 ]; then
    ok "CPU: $CPU_CORES cores"
elif [ "$CPU_CORES" != "?" ]; then
    fail "Minimum 2 CPU cores required (found: $CPU_CORES)"
else
    warn "Could not detect CPU core count"
fi

# RAM
if [ "$PLATFORM" = "linux" ]; then
    TOTAL_RAM_KB=$(awk '/^MemTotal:/ {print $2}' /proc/meminfo 2>/dev/null || echo "0")
    TOTAL_RAM_MB=$((TOTAL_RAM_KB / 1024))
    AVAIL_RAM_KB=$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo 2>/dev/null || echo "0")
    AVAIL_RAM_MB=$((AVAIL_RAM_KB / 1024))
elif [ "$PLATFORM" = "macos" ]; then
    TOTAL_RAM_BYTES=$(sysctl -n hw.memsize 2>/dev/null || echo "0")
    TOTAL_RAM_MB=$((TOTAL_RAM_BYTES / 1024 / 1024))
    AVAIL_RAM_MB=0  # macOS doesn't expose MemAvailable easily
fi

TOTAL_RAM_GB=$(echo "scale=1; $TOTAL_RAM_MB / 1024" | bc 2>/dev/null || echo "?")

if [ "$TOTAL_RAM_MB" -ge 6144 ]; then
    ok "RAM: ${TOTAL_RAM_GB} GB total (semantic-ready)"
    CAN_SEMANTIC=true
elif [ "$TOTAL_RAM_MB" -ge 512 ]; then
    ok "RAM: ${TOTAL_RAM_GB} GB total (rules-only tier)"
    warn "6+ GB RAM recommended for semantic LLM stage"
    CAN_SEMANTIC=false
else
    fail "Minimum 512 MB RAM required (found: ${TOTAL_RAM_GB} GB)"
fi

# Disk space
DISK_FREE_MB=$(df -m "$HOME" 2>/dev/null | awk 'NR==2 {print $4}' || echo "0")
DISK_FREE_GB=$(echo "scale=1; $DISK_FREE_MB / 1024" | bc 2>/dev/null || echo "?")
if [ "$DISK_FREE_MB" -ge 2048 ]; then
    ok "Disk: ${DISK_FREE_GB} GB free"
elif [ "$DISK_FREE_MB" -ge 100 ]; then
    warn "Disk: ${DISK_FREE_GB} GB free (semantic model needs ~1.2 GB)"
else
    fail "Insufficient disk space: ${DISK_FREE_GB} GB free (need 2+ GB)"
fi

# GPU detection
HAS_GPU=false
GPU_VRAM=0

if command -v nvidia-smi &>/dev/null; then
    GPU_NAME=$(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | head -1 || echo "")
    GPU_VRAM=$(nvidia-smi --query-gpu=memory.total --format=csv,noheader,nounits 2>/dev/null | head -1 || echo "0")
    GPU_DRIVER=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader 2>/dev/null | head -1 || echo "?")
    if [ -n "$GPU_NAME" ] && [ "$GPU_VRAM" -ge 2048 ]; then
        ok "GPU: $GPU_NAME (${GPU_VRAM} MB VRAM, driver $GPU_DRIVER)"
        HAS_GPU=true
    elif [ -n "$GPU_NAME" ]; then
        warn "GPU: $GPU_NAME (${GPU_VRAM} MB VRAM — below 2 GB minimum for GPU semantic)"
    fi
elif [ "$PLATFORM" = "macos" ] && system_profiler SPDisplaysDataType 2>/dev/null | grep -q "Apple"; then
    APPLE_CHIP=$(system_profiler SPDisplaysDataType 2>/dev/null | grep "Chip" | head -1 | awk -F: '{print $2}' | xargs)
    ok "GPU: Apple $APPLE_CHIP (Metal, unified memory)"
    HAS_GPU=true
else
    info "No GPU detected — Tier 1 (rules only) or Tier 3 (CPU semantic)"
fi

# glibc (Linux)
if [ "$PLATFORM" = "linux" ]; then
    GLIBC_VER=$(ldd --version 2>&1 | head -1 | grep -oP '[0-9]+\.[0-9]+' | tail -1 || echo "0.0")
    GLIBC_MAJOR=$(echo "$GLIBC_VER" | cut -d. -f1 | tr -cd '0-9')
    GLIBC_MINOR=$(echo "$GLIBC_VER" | cut -d. -f2 | tr -cd '0-9')
    GLIBC_MAJOR=${GLIBC_MAJOR:-0}
    GLIBC_MINOR=${GLIBC_MINOR:-0}
    if [ "$GLIBC_MAJOR" -ge 2 ] 2>/dev/null && [ "$GLIBC_MINOR" -ge 31 ] 2>/dev/null; then
        ok "glibc $GLIBC_VER"
    else
        fail "glibc $GLIBC_VER — requires >= 2.31 (Ubuntu 20.04+, Debian 11+)"
    fi
fi

# Tier recommendation
echo ""
if [ "$HAS_GPU" = true ] && [ "$CAN_SEMANTIC" = true ]; then
    echo -e "${GREEN}${BOLD}  ⚡ System qualifies for: Tier 2 — GPU Semantic (full pipeline)${NC}"
    echo -e "${DIM}     Rule compression (350+ rules, <2ms) + GPU-accelerated semantic (56-75% savings)${NC}"
    RECOMMENDED_TIER="tier2"
elif [ "$CAN_SEMANTIC" = true ]; then
    echo -e "${YELLOW}${BOLD}  ⚡ System qualifies for: Tier 3 — CPU Semantic${NC}"
    echo -e "${DIM}     Rule compression + CPU-based semantic (1-4s latency). Add GPU for Tier 2.${NC}"
    RECOMMENDED_TIER="tier3"
else
    echo -e "${CYAN}${BOLD}  ⚡ System qualifies for: Tier 1 — Rules Only${NC}"
    echo -e "${DIM}     Rule compression only (350+ rules, <2ms, 15-37% savings).${NC}"
    RECOMMENDED_TIER="tier1"
fi

# ═══════════════════════════════════════════════════════
# PHASE 2: System Dependencies
# ═══════════════════════════════════════════════════════
step "Phase 2: System Dependencies"

if [ "$PLATFORM" = "linux" ]; then
    MISSING=""
    command -v cc  >/dev/null 2>&1 || MISSING="build-essential"
    command -v pkg-config >/dev/null 2>&1 || MISSING="$MISSING pkg-config"
    command -v git >/dev/null 2>&1 || MISSING="$MISSING git"

    if [ -n "$MISSING" ]; then
        info "Installing: $MISSING"
        # Try apt update, but don't die on broken third-party repos
        if ! sudo apt update -qq 2>/dev/null; then
            warn "apt update failed (broken third-party repos?). Trying install anyway..."
        fi
        if sudo apt install -y $MISSING 2>/dev/null; then
            ok "System dependencies installed"
        else
            warn "apt install failed. You may need to fix your apt sources first:"
            echo "       sudo rm /etc/apt/sources.list.d/google-chrome.list  # if Chrome repo is broken"
            echo "       sudo apt update && sudo apt install -y $MISSING"
            echo "       Then re-run: bash install.sh"
        fi
    else
        ok "System dependencies present (cc, pkg-config, git)"
    fi

elif [ "$PLATFORM" = "macos" ]; then
    if ! xcode-select -p >/dev/null 2>&1; then
        info "Installing Xcode command line tools..."
        xcode-select --install
        echo "Press Enter after Xcode tools finish installing..."
        read -r
    fi
    ok "Xcode command line tools present"
fi

# ═══════════════════════════════════════════════════════
# PHASE 3: Rust Toolchain
# ═══════════════════════════════════════════════════════
step "Phase 3: Rust Toolchain"

export PATH="$HOME/.cargo/bin:$PATH"

if command -v rustc >/dev/null 2>&1; then
    RUST_VER="$(rustc --version)"
    ok "Rust: $RUST_VER"
else
    info "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # Reload PATH
    if [ -f "$HOME/.cargo/env" ]; then
        source "$HOME/.cargo/env"
    fi
    export PATH="$HOME/.cargo/bin:$PATH"
    if command -v rustc >/dev/null 2>&1; then
        ok "Rust installed: $(rustc --version)"
    else
        fail "Rust installation failed. Install manually: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    fi
fi

# Double-check cargo is available
if ! command -v cargo >/dev/null 2>&1; then
    fail "cargo not found in PATH. Try: source ~/.cargo/env && bash install.sh"
fi

# ═══════════════════════════════════════════════════════
# PHASE 4: Clone & Build
# ═══════════════════════════════════════════════════════
step "Phase 4: Clone & Build"

if [ -d "$INSTALL_DIR/.git" ]; then
    info "Existing install found — pulling latest..."
    cd "$INSTALL_DIR"
    git fetch origin
    git checkout "$BRANCH" 2>/dev/null || true
    git pull origin "$BRANCH" 2>/dev/null || git pull
    ok "Repository updated"
elif [ -d "$INSTALL_DIR" ]; then
    warn "$INSTALL_DIR exists but is not a git repo"
    echo -n "  Remove and re-clone? [y/N] "
    read -r REPLY
    if [[ "$REPLY" =~ ^[Yy]$ ]]; then
        cd "$HOME"
        rm -rf "$INSTALL_DIR"
        git clone -b "$BRANCH" "$REPO" "$INSTALL_DIR" 2>/dev/null || \
        git clone "$REPO" "$INSTALL_DIR"
        ok "Repository cloned"
    else
        info "Using existing directory"
    fi
else
    info "Cloning from GitHub (branch: $BRANCH)..."
    git clone -b "$BRANCH" "$REPO" "$INSTALL_DIR" 2>/dev/null || \
    git clone "$REPO" "$INSTALL_DIR"
    ok "Repository cloned to $INSTALL_DIR"
fi
cd "$INSTALL_DIR"

info "Building Nyquest (release mode — this takes 1-2 minutes)..."
cargo build --release 2>&1 | tail -5

BINARY="$INSTALL_DIR/target/release/nyquest"
if [ ! -f "$BINARY" ]; then
    fail "Build failed — binary not found at $BINARY"
fi

SIZE=$(du -h "$BINARY" | cut -f1)
ok "Build complete: $BINARY ($SIZE)"

# ═══════════════════════════════════════════════════════
# PHASE 5: Semantic Stage (Optional)
# ═══════════════════════════════════════════════════════
step "Phase 5: Semantic LLM Stage"

SETUP_SEMANTIC=false
if [ "$RECOMMENDED_TIER" = "tier2" ] || [ "$RECOMMENDED_TIER" = "tier3" ]; then
    echo ""
    echo -e "  Your system supports the semantic LLM stage (56-75% compression)."
    echo -e "  This installs Ollama + Qwen 2.5 1.5B (~1.2 GB download)."
    echo ""
    echo -n "  Install semantic compression stage? [Y/n] "
    read -r REPLY
    if [[ ! "$REPLY" =~ ^[Nn]$ ]]; then
        SETUP_SEMANTIC=true
    fi
else
    info "Skipping semantic stage (system is Tier 1 — rules only)"
    echo -e "  ${DIM}Add 6+ GB RAM and re-run to enable semantic compression.${NC}"
fi

if [ "$SETUP_SEMANTIC" = true ]; then
    # 5a: Install Ollama
    if command -v ollama &>/dev/null; then
        ok "Ollama already installed"
    else
        info "Installing Ollama..."
        curl -fsSL https://ollama.com/install.sh | sh
        ok "Ollama installed"
    fi

    # 5b: Start Ollama service
    if [ "$PLATFORM" = "linux" ]; then
        if ! systemctl is-active --quiet ollama 2>/dev/null; then
            sudo systemctl start ollama
            sleep 2
        fi
        ok "Ollama service: running"
    elif [ "$PLATFORM" = "macos" ]; then
        # macOS: Ollama runs as a user app
        if ! curl -s "http://localhost:$OLLAMA_PORT" >/dev/null 2>&1; then
            info "Starting Ollama..."
            open -a Ollama 2>/dev/null || warn "Please start Ollama.app manually"
            sleep 3
        fi
    fi

    # 5c: Pull model
    if ollama list 2>/dev/null | grep -q "qwen2.5"; then
        ok "Model $SEMANTIC_MODEL already pulled"
    else
        info "Pulling $SEMANTIC_MODEL (~1.2 GB)..."
        ollama pull "$SEMANTIC_MODEL"
        ok "Model pulled"
    fi

    # 5d: Configure persistent VRAM (Linux)
    if [ "$PLATFORM" = "linux" ]; then
        OVERRIDE_DIR="/etc/systemd/system/ollama.service.d"
        OVERRIDE_FILE="${OVERRIDE_DIR}/nyquest.conf"
        if [ ! -f "$OVERRIDE_FILE" ]; then
            info "Configuring Ollama for persistent VRAM..."
            sudo mkdir -p "$OVERRIDE_DIR"
            sudo tee "$OVERRIDE_FILE" > /dev/null << 'INNEREOF'
[Service]
Environment="OLLAMA_KEEP_ALIVE=-1"
Environment="OLLAMA_NUM_GPU=99"
Environment="OLLAMA_HOST=localhost:11434"
INNEREOF
            sudo systemctl daemon-reload
            sudo systemctl restart ollama
            sleep 3
            ok "OLLAMA_KEEP_ALIVE=-1 configured (model stays in VRAM)"
        else
            ok "Ollama VRAM persistence already configured"
        fi
    fi

    # 5e: Warm up model
    info "Warming up model (first inference loads to GPU)..."
    WARMUP=$(curl -s --connect-timeout 10 --max-time 30 \
        -X POST "http://localhost:${OLLAMA_PORT}/v1/chat/completions" \
        -H "Content-Type: application/json" \
        -d '{"model":"'"${SEMANTIC_MODEL}"'","messages":[{"role":"user","content":"Say OK"}],"max_tokens":5,"temperature":0}' \
        2>/dev/null || echo "FAILED")

    if echo "$WARMUP" | grep -q "choices"; then
        ok "Semantic model responding"
        if command -v nvidia-smi &>/dev/null; then
            VRAM_USED=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null | head -1 || echo "?")
            info "GPU VRAM in use: ${VRAM_USED} MB"
        fi
    else
        warn "Model warm-up failed. Check: journalctl -u ollama -n 20"
    fi

    SEMANTIC_FLAG="--set semantic_enabled=true"
else
    SEMANTIC_FLAG=""
fi

# ═══════════════════════════════════════════════════════
# PHASE 6: Run Preflight Check
# ═══════════════════════════════════════════════════════
step "Phase 6: System Preflight Validation"
"$BINARY" preflight --verbose || true

# ═══════════════════════════════════════════════════════
# PHASE 7: Setup Wizard
# ═══════════════════════════════════════════════════════
step "Phase 7: Configuration"

echo ""
echo -e "${BOLD}${GREEN}  ✓ Nyquest v3.2.0 is built and ready${NC}"
echo ""
echo "  Binary:   $BINARY"
echo "  Config:   $INSTALL_DIR/nyquest.yaml (after setup)"
if [ "$SETUP_SEMANTIC" = true ]; then
    echo "  Semantic: Ollama + $SEMANTIC_MODEL installed"
fi
echo ""
echo -e "  The setup wizard configures providers, compression,"
echo -e "  semantic stage, and optionally installs a systemd service."
echo ""
echo -n "  Launch setup wizard? [Y/n] "
read -r REPLY
if [[ ! "$REPLY" =~ ^[Nn]$ ]]; then
    if [ "$SETUP_SEMANTIC" = true ]; then
        "$BINARY" install $SEMANTIC_FLAG
    else
        "$BINARY" install
    fi
else
    echo ""
    info "Skipped. Run manually later:"
    echo "  cd $INSTALL_DIR"
    echo "  ./target/release/nyquest install"
    echo ""
    info "Or start immediately with defaults:"
    echo "  ./target/release/nyquest serve"
fi

echo ""
echo -e "${BOLD}${GREEN}"
cat << 'DONE'
   ╔═══════════════════════════════════════════════════════╗
   ║  ✓ NYQUEST v3.2.0 INSTALLATION COMPLETE              ║
   ╠═══════════════════════════════════════════════════════╣
   ║  Commands:                                           ║
   ║    nyquest serve        Start the proxy server       ║
   ║    nyquest preflight    Full system check             ║
   ║    nyquest doctor       Quick health check            ║
   ║    nyquest install      Re-run setup wizard           ║
   ║    nyquest configure    Reconfigure settings          ║
   ║    nyquest config show  Show current config           ║
   ╚═══════════════════════════════════════════════════════╝
DONE
echo -e "${NC}"

# ═══════════════════════════════════════════════════════
# PHASE 8: Start Engine
# ═══════════════════════════════════════════════════════
echo -n "  Start Nyquest engine now? [Y/n] "
read -r REPLY
if [[ ! "$REPLY" =~ ^[Nn]$ ]]; then
    echo ""
    # Prefer systemd if the service was installed
    if systemctl --user is-enabled nyquest.service &>/dev/null; then
        systemctl --user start nyquest.service
        sleep 1
        if systemctl --user is-active nyquest.service &>/dev/null; then
            ok "Nyquest started via systemd (port 5400)"
            echo ""
            echo "    Status:  systemctl --user status nyquest"
            echo "    Logs:    journalctl --user -u nyquest -f"
            echo "    Stop:    systemctl --user stop nyquest"
            echo "    Restart: systemctl --user restart nyquest"
        else
            warn "Systemd service failed to start. Check: journalctl --user -u nyquest -n 20"
            info "Try manually: cd $INSTALL_DIR && ./target/release/nyquest serve"
        fi
    else
        # No systemd service — run in background
        cd "$INSTALL_DIR"
        nohup "$BINARY" serve > logs/nyquest.log 2>&1 &
        NYQUEST_PID=$!
        sleep 1
        if kill -0 $NYQUEST_PID 2>/dev/null; then
            ok "Nyquest started in background (PID: $NYQUEST_PID, port 5400)"
            echo "    Logs: tail -f $INSTALL_DIR/logs/nyquest.log"
            echo "    Stop: kill $NYQUEST_PID"
        else
            warn "Failed to start. Check: cat $INSTALL_DIR/logs/nyquest.log"
        fi
    fi
else
    echo ""
    ok "Ready. Start anytime with:"
    echo "    systemctl --user start nyquest"
    echo "    # or: cd $INSTALL_DIR && ./target/release/nyquest serve"
fi

echo ""
ok "Happy compressing! ⚡"

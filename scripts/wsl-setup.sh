#!/bin/bash
set -e

echo "========================================"
echo " Crypto Scalper - WSL2 Setup"
echo "========================================"
echo ""

# Install Rust if not present
if ! command -v cargo &> /dev/null; then
    echo "[1/3] Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
else
    echo "[1/3] Rust already installed."
fi

# Clone or update the repo
REPO_DIR="$HOME/BangidaBOT_AiTradeSupervisor"
if [ -d "$REPO_DIR" ]; then
    echo "[2/3] Updating repository..."
    cd "$REPO_DIR"
    git pull
else
    echo "[2/3] Cloning repository..."
    git clone https://github.com/pen65978/BangidaBOT_AiTradeSupervisor.git "$REPO_DIR"
    cd "$REPO_DIR"
fi

# Build in release mode
echo "[3/3] Building (this may take a few minutes on first run)..."
cargo build --release

echo ""
echo "========================================"
echo " Setup complete!"
echo ""
echo " To start the bot:"
echo "   Double-click scripts/launch.bat from Windows Explorer"
echo ""
echo " To install as a desktop app:"
echo "   1. Start the bot with launch.bat"
echo "   2. Open http://localhost:3000 in Edge or Chrome"
echo "   3. Click the install icon in the address bar"
echo "========================================"

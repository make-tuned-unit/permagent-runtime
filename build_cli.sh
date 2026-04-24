#!/bin/bash
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH"
cargo build --release -p permagent-cli 2>&1 | tail -60
echo "EXIT_CODE=$?"

#!/bin/bash
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/src-tauri"
cargo check 2>&1

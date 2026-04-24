#!/bin/bash
export PATH="/Users/henry/.cargo/bin:$PATH"
cd /Users/henry/.openclaw/workspace/permagent-runtime
cargo check -p goose-server -p goose 2>&1

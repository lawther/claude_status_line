install:
    #!/bin/sh
    set -e
    cargo build --release
    install -m 755 target/release/claude_status_line ~/.claude/statusline
    settings="$HOME/.claude/settings.json"
    sl='{"type":"command","command":"~/.claude/statusline","padding":2}'
    if [ -f "$settings" ]; then
        tmp=$(mktemp)
        jq --argjson sl "$sl" '. + {statusLine: $sl}' "$settings" > "$tmp" && mv "$tmp" "$settings"
    else
        jq -n --argjson sl "$sl" '{statusLine: $sl}' > "$settings"
    fi

check:
    cargo clippy -- -D warnings
    cargo test

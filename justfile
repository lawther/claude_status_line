set windows-shell := ["bash", "-c"]

bin_ext := if os() == "windows" { ".exe" } else { "" }

bold    := `tput bold 2>/dev/null || true`
green   := `tput setaf 2 2>/dev/null || true`
red     := `tput setaf 1 2>/dev/null || true`
reset   := `tput sgr0 2>/dev/null || true`

success := bold + green + "✔︎ "
err     := bold + red + "❌ "

build: lint
    cargo build

build-release: lint
    cargo build --release

install: build-release
    target/release/claude_status_line{{bin_ext}} --install

lint:
    cargo fmt
    cargo clippy -- -D warnings

test:
    cargo test

setup-git-hooks:
    git config core.hooksPath .githooks
    chmod +x .githooks/pre-commit
    @echo "{{success}}Git hooks configured{{reset}}"

precommit:
    #!/usr/bin/env bash
    set -euo pipefail
    tmpfile=$(mktemp)
    staged_list=$(mktemp)
    trap 'rm -f "$tmpfile" "$staged_list"' EXIT
    git diff --cached --name-only -z --diff-filter=d > "$staged_list"
    (
        set -e
        just lint
        xargs -r -0 git add < "$staged_list"
        just test
    ) > "$tmpfile" 2>&1
    status=$?
    if [ $status -ne 0 ]; then
        cat "$tmpfile"
        exit $status
    fi
    echo "{{success}}All pre-commit checks passed{{reset}}"
# BEGIN SHARED RECIPES (DEFAULT) sha256:18670817947290ab
# Generated from agent_rules/snippets/default.just. Do not edit inside this block:
#      edit snippets/default.just, then run `just sync-justfile-recipes` in agent_rules.

# List available recipes
default:
    @just --list

# END SHARED RECIPES (DEFAULT)

set windows-shell := ["bash", "-c"]

bin_ext := if os() == "windows" { ".exe" } else { "" }

# The shared recipes (synced from agent_rules) call this rather than spelling a runner.
uv_run := "uv run"

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

# Point the statusLine config at this dev build for quick local testing.
dev-install: build-release
    target/release/claude_status_line{{bin_ext}} --install --link
    @echo "{{success}}statusLine now points at target/release/claude_status_line{{bin_ext}}{{reset}}"

# Restore the statusLine config to the Homebrew-installed binary.
dev-reset:
    claude_status_line --install --link
    @echo "{{success}}statusLine restored to the Homebrew-installed binary{{reset}}"

lint:
    cargo fmt
    cargo clippy --all-targets -- -D warnings

test:
    cargo test

# One-time setup so cache-expiry alarms can actually show a notification.
# terminal-notifier needs to be launched via `open` once so macOS registers
# it with Launch Services and Notification Center; otherwise permission
# requests never appear and notifications are silently dropped.
setup-cache-alarm:
    brew install terminal-notifier
    open "$(brew --prefix terminal-notifier)/terminal-notifier.app"
    @echo "{{success}}terminal-notifier installed and registered — check System Settings > Notifications if prompts don't appear{{reset}}"

setup-git-hooks:
    git config core.hooksPath .githooks
    chmod +x .githooks/pre-commit
    git config merge.ff only
    git config pull.rebase true
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

# BEGIN SHARED RECIPES (AI_READINESS) sha256:45e10abdfa181601
# Generated from agent_rules/snippets/ai_readiness.just. Do not edit inside this block:
#      edit snippets/ai_readiness.just, then run `just sync-justfile-recipes` in agent_rules.

# Analyse project source files to evaluate AI agent readiness
analyse-ai-readiness *args:
    @{{uv_run}} shared_scripts/analyse_ai_readiness.py {{args}}

# Check source file size thresholds for AI agent readiness (fails if threshold breached)
check-ai-readiness *args:
    @{{uv_run}} shared_scripts/analyse_ai_readiness.py --fail-on {{args}}

# END SHARED RECIPES (AI_READINESS)

# BEGIN SHARED RECIPES (CHECK_HOOKS_DRIFT) sha256:35dc21f648469e76
# Generated from agent_rules/snippets/check_hooks_drift.just. Do not edit inside this block:
#      edit snippets/check_hooks_drift.just, then run `just sync-justfile-recipes` in agent_rules.

# Verify .claude/hooks/ still matches agent_rules/hooks/. Requires the
# agent_rules repo as a sibling checkout; local/pre-commit only for now.
# Wiring this into CI needs agent_rules reachable there — see agent_rules-403.
check-hooks-drift:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ ! -x ../agent_rules/sync_hooks.py ]]; then
        echo "error: ../agent_rules/sync_hooks.py not found -- clone the agent_rules repo as a sibling" >&2
        exit 2
    fi
    if ! command -v uv >/dev/null; then
        echo "error: uv not found -- install uv; ../agent_rules/sync_hooks.py needs it to provision Python 3.13" >&2
        exit 2
    fi
    ../agent_rules/run_shared_script.py --check-failed-exit 3 -- ../agent_rules/sync_hooks.py check .

# END SHARED RECIPES (CHECK_HOOKS_DRIFT)

# BEGIN SHARED RECIPES (LINEAR_HISTORY) sha256:06d5bf7ed4629f17
# Generated from agent_rules/snippets/linear_history.just. Do not edit inside this block:
#      edit snippets/linear_history.just, then run `just sync-justfile-recipes` in agent_rules.

# Sets this repo's own local git config to prefer linear history, overriding whatever
# a clone's global config says. `pull.rebase` and `merge.ff` live in .git/config, which
# git deliberately never lets a repo commit -- so this is the one-off `setup-git-hooks`
# already asks a fresh clone to run, not something a sync can write for you. Actual
# enforcement (for a clone that skips this, or whose global config wins some other way)
# is the `pre-merge-commit` and `pre-push` hooks in .githooks/, which reject merge
# commits regardless of what these two settings say.
setup-linear-history:
    @git config pull.rebase true
    @git config merge.ff only
    @echo "Linear history preferences set for this clone"

# END SHARED RECIPES (LINEAR_HISTORY)

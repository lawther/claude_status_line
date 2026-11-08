# BEGIN FLEET RECIPES (CORE)
# Generated from fleet payload/core/recipes.just. Do not edit inside this block:
#      edit it there, then run `just sync` in fleet.

# List available recipes
default:
    @just --list

# Fail if a file, block or hook registration the fleet manages here has changed since sync
# wrote it. Reads only this repo, so it runs the same in the git hook and in CI.
check-fleet:
    @uv run --script .fleet/core/check_integrity.py

# sync writes the gate list into the next line, so it differs between repos. This repo's own
# precommit depends on it, so a check added to a family reaches the gate of every repo that
# carries the family with the next sync.
#
# Run every check the fleet requires of this repo: the integrity check and each family's gate.
precommit-fleet: check-fleet fleet-gate-core fleet-gate-rules fleet-gate-github

# Fail if the repo's precommit skips precommit-fleet, its CI lacks uv, or its fleet.toml is invalid.
fleet-gate-core:
    @uv run --script .fleet/core/check_recipes_wired.py
    @uv run --script .fleet/core/fleet_config.py

# END FLEET RECIPES (CORE)

set windows-shell := ["bash", "-c"]

bin_ext := if os() == "windows" { ".exe" } else { "" }

# The shared recipes (synced from fleet) call this rather than spelling a runner.
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

precommit: precommit-fleet
    #!/usr/bin/env bash
    set -euo pipefail
    tmpfile=$(mktemp)
    staged_list=$(mktemp)
    trap 'rm -f "$tmpfile" "$staged_list"' EXIT
    status=0
    git diff --cached --name-only -z --diff-filter=d > "$staged_list"
    (
        set -e
        just lint
        xargs -r -0 git add < "$staged_list"
        just test
    ) > "$tmpfile" 2>&1 || status=$?
    if [ $status -ne 0 ]; then
        cat "$tmpfile"
        exit $status
    fi
    echo "{{success}}All pre-commit checks passed{{reset}}"

# BEGIN FLEET RECIPES (SCRIPTS)
# Generated from fleet payload/scripts/recipes.just. Do not edit inside this block:
#      edit it there, then run `just sync` in fleet.

# Analyse project source files to evaluate AI agent readiness
analyse-ai-readiness *args:
    @{{uv_run}} .fleet/scripts/analyse_ai_readiness.py {{args}}

# Check source file size thresholds for AI agent readiness (fails if threshold breached)
check-ai-readiness *args:
    @{{uv_run}} .fleet/scripts/analyse_ai_readiness.py --fail-on {{args}}

# END FLEET RECIPES (SCRIPTS)

# BEGIN FLEET RECIPES (GIT_HOOKS)
# Generated from fleet payload/git_hooks/recipes.just. Do not edit inside this block:
#      edit it there, then run `just sync` in fleet.

# Sets this repo's own local git config to prefer linear history, overriding whatever
# a clone's global config says. `pull.rebase` and `merge.ff` live in .git/config, which
# git deliberately never lets a repo commit -- so this is the one-off `setup-git-hooks`
# already asks a fresh clone to run, not something a sync can write for you. Actual
# enforcement (for a clone that skips this, or whose global config wins some other way)
# is the `pre-merge-commit` and `pre-push` hooks in .fleet/git_hooks/, which reject merge
# commits regardless of what these two settings say.
setup-linear-history:
    @git config pull.rebase true
    @git config merge.ff only
    @echo "Linear history preferences set for this clone"

# Refuses rather than clobbers. git reads exactly one hooks directory, so silently
# overwriting core.hooksPath disables whatever was there -- which is precisely how four
# repos lost their beads integration without a word (ar-x4l). `bd init` does exactly
# this, so the refusal is needed; `check-git-hooks-active` is what reports it if bd wins the
# race anyway. An unset value and the legacy `.githooks` are both safe to replace, and a value is
# judged by the directory it names, so an absolute path to either counts as that directory.
#
# Point git at .fleet/git_hooks/, so `just precommit` runs before each commit here.
setup-git-hooks:
    #!/usr/bin/env bash
    set -euo pipefail
    current=$(git config core.hooksPath || true)
    root=$(git rev-parse --show-toplevel)
    case "$current" in
        /*) named="$current" ;;
        *) named="$root/$current" ;;
    esac
    named="${named%/}"
    if [ -n "$current" ] && [ "$named" != "$root/.githooks" ] && [ "$named" != "$root/.fleet/git_hooks" ]; then
        echo "core.hooksPath is already $current -- refusing to overwrite it." >&2
        echo "git reads one hooks directory only; merge the two before setting this." >&2
        exit 1
    fi
    git config core.hooksPath .fleet/git_hooks
    echo "Git hooks set up"

# END FLEET RECIPES (GIT_HOOKS)

# BEGIN FLEET RECIPES (GITHUB)
# Generated from fleet payload/github/recipes.just. Do not edit inside this block:
#      edit it there, then run `just sync` in fleet.

# zizmor's version is fleet's locked one, stamped in by the sync, so update-deps there
# upgrades it fleet-wide. --offline keeps the audit reproducible and token-free in the git hook;
# known-vulnerable actions reach the fleet through Dependabot security updates instead. zizmor
# exits 3 when it collects no input, which is a repo with no workflows, actions or dependabot.yml.
#
# Audit this repo's GitHub Actions workflows, composite actions and dependabot.yml with zizmor.
lint-workflows:
    #!/usr/bin/env bash
    set -uo pipefail
    status=0
    uvx zizmor@1.32.0 --offline . || status=$?
    if [ "$status" -eq 3 ]; then
        echo "lint-workflows: nothing to audit"
        exit 0
    fi
    exit "$status"

# Exemptions from these checks are the repo's own, in its fleet.toml.
#
# Fail if a workflow fails zizmor, an action is not pinned to a commit SHA, or dependabot.yml can open version updates.
fleet-gate-github: lint-workflows
    @uv run --script .fleet/github/check_actions_pinned.py
    @uv run --script .fleet/github/check_dependabot_security.py

# END FLEET RECIPES (GITHUB)

# BEGIN FLEET RECIPES (RULES)
# Generated from fleet payload/rules/recipes.just. Do not edit inside this block:
#      edit it there, then run `just sync` in fleet.

# Fail if a generated Beads integration block has reappeared in the repo's root markdown.
fleet-gate-rules:
    @uv run --script .fleet/rules/check_beads_block.py

# END FLEET RECIPES (RULES)

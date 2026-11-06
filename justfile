# BEGIN FLEET RECIPES (CORE)
# Generated from agent_rules payload/core/recipes.just. Do not edit inside this block:
#      edit it there, then run `just sync` in agent_rules.

# List available recipes
default:
    @just --list

# Fail if a file, block or hook registration the fleet manages here has changed since sync
# wrote it. Reads only this repo, so it runs the same in the git hook and in CI.
check-fleet:
    @uv run --script .fleet/core/check_integrity.py

# Every check the fleet requires of this repo. This repo's own precommit depends on it, so a
# check added here reaches every repo's gate with the next sync.
precommit-fleet: check-fleet

# Land this repo's open Dependabot PRs on main, one tested commit per PR. Never pushes.
apply-dependabot *args:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ ! -x ../agent_rules/apply_dependabot.py ]]; then
        echo "error: ../agent_rules/apply_dependabot.py not found -- clone the agent_rules repo as a sibling" >&2
        exit 2
    fi
    ../agent_rules/run_shared_script.py --check-failed-exit 3 -- ../agent_rules/apply_dependabot.py . {{args}}

# Apply this repo's Dependabot security fixes, then upgrade every other dependency locally,
# one tested commit each. Never pushes.
update-deps *args:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ ! -x ../agent_rules/update_deps.py ]]; then
        echo "error: ../agent_rules/update_deps.py not found -- clone the agent_rules repo as a sibling" >&2
        exit 2
    fi
    ../agent_rules/run_shared_script.py --check-failed-exit 3 -- ../agent_rules/update_deps.py . {{args}}

# END FLEET RECIPES (CORE)

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
    just lint-pep723
    just lint-workflows

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
# Generated from agent_rules payload/scripts/recipes.just. Do not edit inside this block:
#      edit it there, then run `just sync` in agent_rules.

# Analyse project source files to evaluate AI agent readiness
analyse-ai-readiness *args:
    @{{uv_run}} shared_scripts/analyse_ai_readiness.py {{args}}

# Check source file size thresholds for AI agent readiness (fails if threshold breached)
check-ai-readiness *args:
    @{{uv_run}} shared_scripts/analyse_ai_readiness.py --fail-on {{args}}

# Type-check the synced PEP 723 scripts in shared_scripts/ in an ephemeral env built from each
# script's own inline metadata. ty resolves imports against the project's .venv and has no
# notion of PEP 723 isolation, so checking them via the project's own `ty check` fails or
# silently passes depending on whether the .venv happens to carry the same packages. Keep
# shared_scripts/ out of the project's own `ty check` invocation, and call this recipe from
# the project's lint recipe. Drop both once astral-sh/ty#691 lands.
lint-pep723:
    @uvx --with-requirements shared_scripts/analyse_ai_readiness.py ty check shared_scripts/analyse_ai_readiness.py

# END FLEET RECIPES (SCRIPTS)

# BEGIN FLEET RECIPES (GIT_HOOKS)
# Generated from agent_rules payload/git_hooks/recipes.just. Do not edit inside this block:
#      edit it there, then run `just sync` in agent_rules.

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

# END FLEET RECIPES (GIT_HOOKS)

# BEGIN FLEET RECIPES (GITHUB)
# Generated from agent_rules payload/github/recipes.just. Do not edit inside this block:
#      edit it there, then run `just sync` in agent_rules.

# zizmor's version is agent_rules' locked one, stamped in by the sync, so update-deps there
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

# END FLEET RECIPES (GITHUB)

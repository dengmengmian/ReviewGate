#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

workflow="$ROOT/integrations/github-action/example-workflow.yml"
config="$ROOT/reviewgate.toml.example"
readmes=("$ROOT/README.md" "$ROOT/README.en.md")
action="$ROOT/integrations/github-action/action.yml"
issue_action="$ROOT/integrations/github-action/issue/action.yml"
issue_workflow="$ROOT/integrations/github-action/example-issue-workflow.yml"
install_sh="$ROOT/install.sh"
install_ps1="$ROOT/install.ps1"
cli_main="$ROOT/crates/cli/src/main.rs"

grep -q 'uses: dengmengmian/ReviewGate/integrations/github-action@v0' "$workflow"

if grep -Eq '^[[:space:]]*api_key[[:space:]]*=' "$config"; then
  echo "reviewgate.toml.example must not contain an active api_key value" >&2
  exit 1
fi

grep -q 'REVIEWGATE_API_KEY' "$config"

grep -q 'sha256sum.txt' "$install_sh"
grep -q 'shasum -a 256 -c' "$install_sh"
grep -q 'sha256sum.txt' "$install_ps1"
grep -q 'Get-FileHash' "$install_ps1"
grep -q 'verify_release_checksum' "$cli_main"

grep -q 'ARGS=(' "$action"
grep -q 'reviewgate review "${ARGS\[@\]}"' "$action"
if grep -q 'reviewgate review \$ARGS' "$action"; then
  echo "GitHub Action must execute ReviewGate with a bash array, not string-split ARGS" >&2
  exit 1
fi

grep -q 'GITHUB_ACTION_PATH/../../../install.sh' "$issue_action"
grep -q 'ARGS=(issue review' "$issue_action"
grep -q 'reviewgate "${ARGS\[@\]}"' "$issue_action"
grep -q 'issues:' "$issue_workflow"
grep -q '!github.event.issue.pull_request' "$issue_workflow"
if grep -q 'watch --publish' "$issue_workflow"; then
  echo "example issue workflow must not recommend stateless watch --publish" >&2
  exit 1
fi

for readme in "${readmes[@]}"; do
  if grep -Eq '^[[:space:]]*api_key[[:space:]]*=' "$readme"; then
    echo "$(basename "$readme") must not show an active api_key in quick config examples" >&2
    exit 1
  fi
done

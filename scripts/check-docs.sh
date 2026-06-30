#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

workflow="$ROOT/integrations/github-action/example-workflow.yml"
config="$ROOT/reviewgate.toml.example"
readmes=("$ROOT/README.md" "$ROOT/README.en.md")

grep -q 'uses: dengmengmian/ReviewGate/integrations/github-action@v0.2.0' "$workflow"

if grep -Eq '^[[:space:]]*api_key[[:space:]]*=' "$config"; then
  echo "reviewgate.toml.example must not contain an active api_key value" >&2
  exit 1
fi

grep -q 'REVIEWGATE_API_KEY' "$config"

for readme in "${readmes[@]}"; do
  if grep -Eq '^[[:space:]]*api_key[[:space:]]*=' "$readme"; then
    echo "$(basename "$readme") must not show an active api_key in quick config examples" >&2
    exit 1
  fi
done

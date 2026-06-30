#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

workflow="$ROOT/integrations/github-action/example-workflow.yml"
config="$ROOT/reviewgate.toml.example"

grep -q 'uses: dengmengmian/ReviewGate/integrations/github-action@v0.1.4' "$workflow"

if grep -Eq '^[[:space:]]*api_key[[:space:]]*=' "$config"; then
  echo "reviewgate.toml.example must not contain an active api_key value" >&2
  exit 1
fi

grep -q 'REVIEWGATE_API_KEY' "$config"

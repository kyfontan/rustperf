#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"

if [[ "${OSTYPE:-}" == darwin* ]]; then
  VSCODE_USER_DIR="${HOME}/Library/Application Support/Code/User"
else
  VSCODE_USER_DIR="${HOME}/.config/Code/User"
fi

mkdir -p "$CARGO_HOME_DIR"
mkdir -p "$VSCODE_USER_DIR/snippets"

cargo install cargo-dylint dylint-link

CONFIG_FILE="$CARGO_HOME_DIR/config.toml"
if [[ ! -f "$CONFIG_FILE" ]]; then
  cat > "$CONFIG_FILE" <<'CFG'
[alias]
pc = "clippy --workspace --all-targets --all-features -- -D warnings -W clippy::pedantic -W clippy::perf -D clippy::linkedlist -D clippy::vec_box -D clippy::ptr_arg"
pd = "dylint --all"
CFG
else
  if ! grep -q '^pc = ' "$CONFIG_FILE"; then
    cat >> "$CONFIG_FILE" <<'CFG'

[alias]
pc = "clippy --workspace --all-targets --all-features -- -D warnings -W clippy::pedantic -W clippy::perf -D clippy::linkedlist -D clippy::vec_box -D clippy::ptr_arg"
pd = "dylint --all"
CFG
  fi
fi

cp "$ROOT_DIR/snippets/rust.json" "$VSCODE_USER_DIR/snippets/rust.json"

echo
printf 'VS Code snippet installed to:\n  %s/snippets/rust.json\n\n' "$VSCODE_USER_DIR"
printf 'Add this to projects that should load the custom lints:\n\n'
cat <<SNIPPET
[workspace.metadata.dylint]
libraries = [
  { path = "$ROOT_DIR/machine-oriented-lints" },
]

[machine_oriented_lints]
small_vec_capacity_threshold = 64
vec_new_then_push_min_pushes = 2
SNIPPET

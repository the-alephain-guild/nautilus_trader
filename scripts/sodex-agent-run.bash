#!/usr/bin/env bash
# Runs a command with the SoDEX API key injected from the macOS keychain, so the key never
# appears in a command line, in shell history, or in the transcript of an agent session.
#
# One-time setup. Passing `-w` last makes `security` prompt for the key, which keeps it out of
# argv and out of history:
#
#   security add-generic-password -U -a perps-agent-01 -s sodex-perps-agent-01 \
#     -D "SoDEX API key" -w
#
# The `-a` account field holds the name the key is registered under at the venue, and this
# script passes it through as SODEX_API_KEY_NAME whenever that variable is unset. The two
# travel together on purpose: a correct key used under the wrong name is answered with
# `API key not found`, an error that names credentials for what is really a mismatch.
#
# Usage:
#
#   scripts/sodex-agent-run.bash cargo run -q -p nautilus-sodex --example place_modify_cancel
#   SODEX_KEYCHAIN_SERVICE=sodex-spot-agent-01 scripts/sodex-agent-run.bash <command>
#
# What this does and does not protect. The key travels keychain -> this script -> the child's
# environment, so it stays out of every durable record. It remains readable by any process of
# this user while the child runs, because `ps -E` shows a process its environment. So this
# bounds where the key is *written down*, not who can reach it on this machine. The controls
# for reach are elsewhere and belong with it: a key registered for this purpose alone, an
# expiry the venue enforces, and a revoke path that has been exercised before it is needed
# (`cargo run -p nautilus-sodex --example revoke_api_key`).

set -euo pipefail

service=${SODEX_KEYCHAIN_SERVICE:-sodex-perps-agent-01}

if (($# == 0)); then
  echo "Usage: scripts/sodex-agent-run.bash COMMAND [ARGS]..." >&2
  exit 2
fi

if ! key=$(security find-generic-password -s "$service" -w 2> /dev/null); then
  echo "No keychain item for service '$service'." >&2
  echo "Create one with:" >&2
  echo "  security add-generic-password -U -a <venue key name> -s $service -w" >&2
  exit 1
fi

name=${SODEX_API_KEY_NAME:-}
if [[ -z $name ]]; then
  name=$(security find-generic-password -s "$service" 2> /dev/null |
    sed -n 's/^[[:space:]]*"acct"<blob>="\(.*\)"$/\1/p')
fi

if [[ -z $name ]]; then
  echo "Keychain item '$service' carries no account name, and SODEX_API_KEY_NAME is unset." >&2
  echo "Re-add it with '-a <venue key name>', or set SODEX_API_KEY_NAME for this run." >&2
  exit 1
fi

exec env SODEX_API_PRIVATE_KEY="$key" SODEX_API_KEY_NAME="$name" "$@"

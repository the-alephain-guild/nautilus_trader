#!/usr/bin/env bash
# Runs a command with the SoDEX API key injected from the macOS keychain, so the key never
# appears in a command line, in shell history, or in the transcript of an agent session.
#
# One-time setup. The key is 64 hex characters, and it must arrive intact: the parser wants
# exactly 64 after an optional 0x, so a paste that drops one character fails later with
# `KeyLength(63)` - at the venue, several steps away from the mistake.
#
# Typing it into `security ... -w` at the prompt works, and the prompt is where the secret goes
# despite calling itself a password, but a hand paste is exactly what loses a character. So pass
# it through the clipboard with a gate that refuses to store the wrong length:
#
#   K=$(pbpaste | tr -d '[:space:]')
#   if [ ${#K} -eq 64 ]; then
#     printf '%s\n%s\n' "$K" "$K" | security add-generic-password -U \
#       -a perps-agent-01 -s sodex-perps-agent-01 -D "SoDEX API key" -w && echo stored
#   else echo "clipboard is ${#K} chars, expected 64 - not stored"; fi
#   unset K
#
# The key reaches `security` through a variable and stdin, never through argv, so it stays out of
# the process list; a leading space on the line keeps it out of shell history where that is on.
#
# Then check what was stored, printing its shape and never its content:
#
#   K=$(security find-generic-password -s sodex-perps-agent-01 -w)
#   printf 'length=%s hex=%s\n' "${#K}" \
#     "$(printf '%s' "$K" | grep -qE '^[0-9a-fA-F]{64}$' && echo yes || echo no)"
#   unset K
#
# `length=64 hex=yes` is what to expect. Then prove the venue accepts it, which the shape cannot:
#
#   SODEX_ACCOUNT_ID=<id> SODEX_MARKET=perps \
#     scripts/sodex-agent-run.bash cargo run -q -p nautilus-sodex --example verify_signing
#
# That sends `scheduleCancel` with no timestamp - idempotent, weight 1, touching no order - so it
# answers only whether this key can sign on this engine.
#
# The `-a` account field must hold the name the key is registered under at the venue, and this
# script passes it through as SODEX_API_KEY_NAME whenever that variable is unset. The two travel
# together on purpose: a correct key used under the wrong name is answered with
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

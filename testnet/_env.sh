#!/usr/bin/env bash
# Shared helpers for the testnet scripts. Source it, don't run it.
set -euo pipefail

readonly HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly ROOT="$(cd "$HERE/.." && pwd)"
readonly EXT="$ROOT/external-contracts"
readonly STATE="$ROOT/testnet.contracts.json"
readonly NET="${NET:-testnet}"
readonly LOG="$ROOT/demo/rwa-yield-sharing/run-log.md"

# Identities in the local stellar CLI config (created with `stellar keys generate ts-<name> --fund`).
readonly ADMIN_ID="${ADMIN_ID:-ts-admin}"       # strategy admin, vault manager and rebalance manager
readonly KEEPER_ID="${KEEPER_ID:-ts-keeper}"    # strategy keeper (invest / divest)
readonly LP_ID="${LP_ID:-ts-lp}"                # pool liquidity provider and arbitrageur
readonly PARTNER_ID="${PARTNER_ID:-ts-partner}" # partner app: the vault fee receiver
readonly USERS=(ts-alice ts-bob)

# Token and pricing constants. Both tokens have 7 decimals; the oracle reports 14.
readonly ONE=10000000
readonly ORACLE_DECIMALS=14
readonly ORACLE_ONE=100000000000000
readonly ORACLE_RESOLUTION_MS=3600000           # one price slot per hour, same as the anchor window
readonly ORACLE_TTL=86400

readonly DEFINDEX_FACTORY="CDSCWE4GLNBYYTES2OCYDFQA2LLY4RBIAX6ZI32VSUXD7GO6HRPO4A32"
readonly SOROSWAP_ROUTER="CCJUD55AG6W5HAI5LRVNKAE5WDP5XGZBUDS5WNTIVDU7O264UZZE7BRD"

die() { echo "error: $*" >&2; exit 1; }
log() { echo "→ $*" >&2; }
addr() { stellar keys address "$1"; }

[[ -f "$STATE" ]] || echo '{}' > "$STATE"
state_get() { jq -r --arg k "$1" '.[$k] // empty' "$STATE"; }
state_set() { local t; t="$(mktemp)"; jq --arg k "$1" --arg v "$2" '.[$k]=$v' "$STATE" > "$t" && mv "$t" "$STATE"; }

# Testnet RPC sometimes drops a submission; retry a few times.
retry() { local n=0; until "$@"; do n=$((n+1)); [[ $n -ge 3 ]] && return 1; sleep 3; done; }

# inv <contract> <signer> -- <fn> [args...]   submits and prints the return value
inv() { local id="$1" src="$2"; shift 2; retry stellar contract invoke --id "$id" --source-account "$src" --network "$NET" "$@"; }
# view <contract> -- <fn> [args...]           simulation only
view() { local id="$1"; shift; stellar contract invoke --id "$id" --source-account "$ADMIN_ID" --network "$NET" --send no "$@"; }

# Appends a line to the demo run log.
record() { mkdir -p "$(dirname "$LOG")"; echo "$*" >> "$LOG"; }

# Reflector wants timestamps in ms aligned to its resolution.
aligned_ms() { echo $(( ( $(date +%s) * 1000 / ORACLE_RESOLUTION_MS ) * ORACLE_RESOLUTION_MS )); }

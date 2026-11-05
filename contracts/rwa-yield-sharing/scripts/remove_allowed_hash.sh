#!/usr/bin/env bash
#
# Remove a depositor code hash from a deployed RWA yield-sharing strategy's allowlist.
#
# Blocks only *future* deposits from contracts running that code — it never affects
# withdrawals, so a de-listed vault can always exit funds it already holds.
#
# Accepts the hash directly (64 hex chars) OR a path to a .wasm file (hashed for you).
#
# Reads the strategy address from <workspace_root>/<network>.contracts.json
# (key "rwa-yield-sharing[-<DEPLOYMENT>]"); prompts if the file/key is missing.
# Honors DEPLOYMENT=<ustry|usdy|dejtrsy> (unset = the default instance).
#
# Usage:
#   Interactive:  ./remove_allowed_hash.sh
#   Positional:   ./remove_allowed_hash.sh <network> <source_account> <hash_or_wasm_path>
#
# The signer MUST be the strategy ADMIN (remove_allowed_hash calls admin.require_auth()).

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"

banner "REMOVE ALLOWED HASH"

NETWORK=$(resolve_with_default "Network (testnet/mainnet)" "testnet" "${1-__UNSET__}")
select_network "$NETWORK"

SOURCE_ACCOUNT=$(resolve_required "Signer identity (must be the strategy admin)" "${2-__UNSET__}")

ensure_network
ensure_signer
load_strategy_id
echo

HASH_INPUT=$(resolve_required "Code hash (64 hex) or path to a .wasm file" "${3-__UNSET__}")
HASH=$(resolve_wasm_hash "$HASH_INPUT")

INCLUSION_FEE="${INCLUSION_FEE:-$DEFAULT_INCLUSION_FEE}"
require_positive_i128 "inclusion fee" "$INCLUSION_FEE"

echo
echo "──────────────────────────────────────"
echo " network:    $NETWORK"
echo " signer:     $SOURCE_ACCOUNT ($SIGNER_PUBKEY)"
echo " strategy:   $STRATEGY_ID ($STRATEGY_KEY)"
echo " code hash:  $HASH"
[[ "$HASH_INPUT" != "$HASH" && -f "$HASH_INPUT" ]] && echo " (from wasm: $HASH_INPUT)"
echo " inclusion fee:  $INCLUSION_FEE stroops"
echo "──────────────────────────────────────"

read -rp "Remove this code hash now? [y/N] " confirm
[[ "$confirm" =~ ^[yY]$ ]] || { echo "aborted"; exit 0; }

stellar contract invoke \
  --id "$STRATEGY_ID" \
  --source-account "$SOURCE_ACCOUNT" \
  --network "$NETWORK" \
  --inclusion-fee "$INCLUSION_FEE" \
  -- remove_allowed_hash \
  --hash "$HASH"

echo
echo "✅ Removed code hash $HASH"

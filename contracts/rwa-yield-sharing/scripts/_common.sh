#!/usr/bin/env bash
#
# Shared helpers for the RWA yield-sharing strategy operational scripts (deposit / withdraw /
# add_depositor / remove_depositor). Source it — don't run it:
#
#   source "$(dirname "${BASH_SOURCE[0]}")/_common.sh"
#
# It provides the network constants, the prompt/resolve helpers, a signer + XLM
# balance check, and strategy-id resolution from <workspace_root>/<network>.contracts.json.
# Factored out so the action scripts stay short and there is one place to fix a bug.

# --- Network constants ---

readonly MAINNET_RPC_URL="https://rpc.lightsail.network"
readonly MAINNET_PASSPHRASE="Public Global Stellar Network ; September 2015"
readonly MAINNET_XLM_SAC="CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"

readonly TESTNET_RPC_URL="https://soroban-testnet.stellar.org"
readonly TESTNET_PASSPHRASE="Test SDF Network ; September 2015"
readonly TESTNET_XLM_SAC="CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC"

# Inclusion-fee bid in stroops. The CLI default (100) is below the network
# minimum during mainnet fee surges and fails with txINSUFFICIENT_FEE even
# though the account is well funded. 0.1 XLM gives ample headroom; the
# resource fee is computed from simulation and added on top of this.
# Override for exceptional congestion with:  INCLUSION_FEE=<stroops> ./<script>.sh
readonly DEFAULT_INCLUSION_FEE=1000000

# Which deployment the script targets. The default instance uses the bare key; every other
# instance is keyed "rwa-yield-sharing-<deployment>" in <network>.contracts.json.
#   DEPLOYMENT=ustry ./invest.sh testnet keeper 100000000000
# Unset = the default instance.
readonly DEPLOYMENT="${DEPLOYMENT:-}"
readonly STRATEGY_KEY="rwa-yield-sharing${DEPLOYMENT:+-$DEPLOYMENT}"

# Repo root = two levels above this scripts/ dir (contracts/rwa-yield-sharing/scripts).
readonly COMMON_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly WORKSPACE_ROOT="$(cd "$COMMON_DIR/../../.." && pwd)"

# --- Generic helpers ---

die() { echo "error: $*" >&2; exit 1; }

# Draw a title box sized to its text. The strategy name is long enough that a fixed-width
# box overflows, so size it here instead of hand-padding
# nine scripts. Width is counted in characters, not bytes, so the box lines up despite the
# multi-byte '·' separator.
banner() {
  local text="  RWA Yield-Sharing · $1  "
  local width=${#text} line=""
  local i
  for (( i = 0; i < width; i++ )); do line+="═"; done
  printf '╔%s╗\n║%s║\n╚%s╝\n' "$line" "$text" "$line"
}

prompt_required() {
  local label="$1" answer
  while true; do
    read -rp "$label: " answer
    [[ -n "$answer" ]] && { printf '%s' "$answer"; return; }
    echo "  value required" >&2
  done
}

prompt_default() {
  local label="$1" default="$2" answer
  read -rp "$label [$default]: " answer
  printf '%s' "${answer:-$default}"
}

resolve_with_default() {
  local label="$1" default="$2" supplied="${3-__UNSET__}"
  if [[ "$supplied" == "__UNSET__" ]]; then
    prompt_default "$label" "$default"
  elif [[ -z "$supplied" || "$supplied" == "-" ]]; then
    printf '%s' "$default"
  else
    printf '%s' "$supplied"
  fi
}

resolve_required() {
  local label="$1" supplied="${2-__UNSET__}"
  if [[ "$supplied" == "__UNSET__" ]]; then
    prompt_required "$label"
  else
    [[ -n "$supplied" && "$supplied" != "-" ]] || die "$label is required"
    printf '%s' "$supplied"
  fi
}

require_positive_i128() {
  local label="$1" value="$2"
  # Positive integer, no leading zero. The contract rejects <= 0; the i128 upper
  # bound is left to the network to enforce.
  [[ "$value" =~ ^[1-9][0-9]*$ ]] \
    || die "$label must be a positive integer (stroops), got '$value'"
}

# Resolve a 32-byte contract code hash (64 lowercase hex chars, the form the CLI wants for
# a BytesN<32> arg) from EITHER a hex string OR a path to a .wasm file. A Soroban contract's
# on-ledger code hash is sha256(wasm), so hashing the wasm file yields the same value the
# strategy reads via `Address::executable()` for a contract deployed from it.
# Echoes the hash on success; dies on anything invalid.
resolve_wasm_hash() {
  local input="$1" h
  if [[ "$input" =~ ^[0-9a-fA-F]{64}$ ]]; then
    # Already a hash — normalize to lowercase.
    printf '%s' "$input" | tr 'A-F' 'a-f'
    return
  fi
  if [[ -f "$input" ]]; then
    if command -v sha256sum >/dev/null 2>&1; then
      h="$(sha256sum "$input" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
      h="$(shasum -a 256 "$input" | awk '{print $1}')"
    else
      die "need 'sha256sum' or 'shasum' to hash '$input'"
    fi
    printf '%s' "$h"
    return
  fi
  die "expected a 64-char code hash (hex) or a path to a .wasm file, got '$input'"
}

# --- OracleSource SCVal JSON builders ---
#
# Shared by deploy.sh (building the constructor's `oracles` arg) and propose_oracles.sh
# (building `--oracles`). Verified 2026-09-22 by round-tripping through
# `stellar xdr encode --type ScVal` / `stellar xdr decode`: a #[contracttype] struct is a map
# with symbol keys sorted alphabetically (decimals, kind, ttl_seconds for OracleSource); a
# tuple enum variant is a vec [symbol, fields...]. Field order mirrors storage.rs OracleKind
# exactly: Sep40(feed, asset), RedStone(adapter).
sep40_source() { # feed rwa ttl decimals
  printf '{"map":[{"key":{"symbol":"decimals"},"val":{"u32":%s}},{"key":{"symbol":"kind"},"val":{"vec":[{"symbol":"Sep40"},{"address":"%s"},{"address":"%s"}]}},{"key":{"symbol":"ttl_seconds"},"val":{"u64":"%s"}}]}' "$4" "$1" "$2" "$3"
}
redstone_source() { # adapter ttl
  printf '{"map":[{"key":{"symbol":"decimals"},"val":{"u32":8}},{"key":{"symbol":"kind"},"val":{"vec":[{"symbol":"RedStone"},{"address":"%s"}]}},{"key":{"symbol":"ttl_seconds"},"val":{"u64":"%s"}}]}' "$1" "$2"
}

run_with_spinner() {
  local label="$1"; shift
  local tmp rc_file rc
  tmp="$(mktemp)"
  rc_file="$(mktemp)"

  ( "$@" >"$tmp" 2>&1; echo $? >"$rc_file" ) &
  local pid=$!

  local chars='|/-\' i=0
  while kill -0 "$pid" 2>/dev/null; do
    printf '\r  %s %s' "${chars:$((i++ % ${#chars})):1}" "$label" >&2
    sleep 0.1
  done
  wait "$pid" 2>/dev/null || true
  printf '\r\033[K' >&2

  rc="$(cat "$rc_file")"
  rm -f "$rc_file"
  cat "$tmp"
  rm -f "$tmp"
  return "${rc:-1}"
}

# --- Network / signer / strategy resolution ---
#
# select_network <name> sets NETWORK + the three network-scoped vars, or dies.
select_network() {
  NETWORK="$1"
  case "$NETWORK" in
    mainnet)
      NETWORK_RPC_URL="$MAINNET_RPC_URL"
      NETWORK_PASSPHRASE="$MAINNET_PASSPHRASE"
      XLM_SAC_ID="$MAINNET_XLM_SAC"
      ;;
    testnet)
      NETWORK_RPC_URL="$TESTNET_RPC_URL"
      NETWORK_PASSPHRASE="$TESTNET_PASSPHRASE"
      XLM_SAC_ID="$TESTNET_XLM_SAC"
      ;;
    *) die "unknown network: '$NETWORK' (expected 'testnet' or 'mainnet')" ;;
  esac
}

ensure_network() {
  if stellar network ls 2>/dev/null | grep -qw "$NETWORK"; then
    echo "✓ network '$NETWORK' configured"
  else
    echo "→ adding network '$NETWORK'"
    stellar network add "$NETWORK" \
      --rpc-url "$NETWORK_RPC_URL" \
      --network-passphrase "$NETWORK_PASSPHRASE"
  fi
}

# Verifies the signer identity exists and has XLM. Sets SIGNER_PUBKEY.
ensure_signer() {
  if ! stellar keys ls 2>/dev/null | grep -qw "$SOURCE_ACCOUNT"; then
    die "identity '$SOURCE_ACCOUNT' not found. Create one first with:
    stellar keys generate $SOURCE_ACCOUNT --network $NETWORK
    stellar keys add $SOURCE_ACCOUNT --secret-key
  Then fund the account with XLM before re-running this script."
  fi

  SIGNER_PUBKEY="$(stellar keys public-key "$SOURCE_ACCOUNT")"
  echo "✓ identity '$SOURCE_ACCOUNT' → $SIGNER_PUBKEY"

  local balance_output balance_stroops balance_xlm
  if ! balance_output="$(run_with_spinner "checking XLM balance..." \
    stellar contract invoke \
      --id "$XLM_SAC_ID" \
      --source-account "$SOURCE_ACCOUNT" \
      --network "$NETWORK" \
      --send no \
      -- balance --id "$SIGNER_PUBKEY")"; then
    die "failed to fetch XLM balance for $SIGNER_PUBKEY:
$balance_output"
  fi

  balance_stroops="${balance_output//\"/}"
  [[ -n "$balance_stroops" && "$balance_stroops" != "0" ]] \
    || die "signer has 0 XLM on $NETWORK. Fund $SIGNER_PUBKEY before retrying."

  balance_xlm="$(awk -v s="$balance_stroops" 'BEGIN { printf "%.7f", s / 10000000 }')"
  echo "  balance: $balance_xlm XLM ($balance_stroops stroops)"
}

# Resolves STRATEGY_ID from <workspace_root>/<network>.contracts.json (key STRATEGY_KEY),
# falling back to an interactive prompt if the file, jq, or key is missing.
load_strategy_id() {
  local contracts_file="$WORKSPACE_ROOT/$NETWORK.contracts.json"
  if [[ -f "$contracts_file" ]] && command -v jq >/dev/null 2>&1; then
    STRATEGY_ID="$(jq -r --arg k "$STRATEGY_KEY" '.[$k] // empty' "$contracts_file")"
    if [[ -n "$STRATEGY_ID" ]]; then
      echo "✓ $STRATEGY_KEY ($NETWORK): $STRATEGY_ID  (from $contracts_file)"
      return
    fi
    echo "⚠  '$STRATEGY_KEY' not found in $contracts_file"
  fi
  STRATEGY_ID=$(prompt_required "RWA yield-sharing strategy contract id")
}

# Resolves OTC_PRICE for invest_otc / divest_otc from a positional arg or a prompt. The price
# is the one agreed with the counterparty, in the unit of get_pricing_state().rate: USDC
# stroops per whole RWA times 1e12. The literal "rate" means "fill at the current
# gated rate". It previews the distance from the rate in bps and refuses locally when that is
# outside the stored otc_max_deviation_bps band; the contract checks the same band on-chain
# and is the authority (the rate can move before the tx lands, and at the shipped band 0 any
# tick in between makes the fill revert). JSON goes through python3, not jq: the values exceed
# bash's 64-bit arithmetic and jq < 1.7 rounds large numbers.
_otc_json_field() {
  python3 -c 'import json,sys
try:
    v = json.loads(sys.argv[1]).get(sys.argv[2])
    print("" if v is None else int(v))
except Exception:
    print("")' "$1" "$2"
}

resolve_otc_price() {
  local positional="$1" state params rate="" band=""
  command -v python3 >/dev/null 2>&1 || die "python3 is required for OTC fills (big-integer rate math)"
  state="$(stellar contract invoke \
    --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
    -- get_pricing_state 2>/dev/null || true)"
  params="$(stellar contract invoke \
    --id "$STRATEGY_ID" --source-account "$SOURCE_ACCOUNT" --network "$NETWORK" --send no \
    -- get_params 2>/dev/null || true)"
  if [[ -n "$state" ]]; then
    echo "→ current pricing state:"
    echo "$state"
    rate="$(_otc_json_field "$state" rate)"
  fi
  [[ -n "$params" ]] && band="$(_otc_json_field "$params" otc_max_deviation_bps)"
  OTC_PRICE=$(resolve_required "price (rate units: USDC stroops per whole RWA x 1e12, or 'rate')" "$positional")
  if [[ "$OTC_PRICE" == "rate" ]]; then
    [[ -n "$rate" ]] || die "could not read the current rate; pass the price explicitly"
    OTC_PRICE="$rate"
  fi
  require_positive_i128 "price" "$OTC_PRICE"
  if [[ -z "$rate" || "$rate" == "0" || -z "$band" ]]; then
    echo "⚠  could not read the rate or the band; skipping the local band check (the contract still enforces it)"
    return
  fi
  local verdict
  verdict="$(python3 - "$rate" "$OTC_PRICE" "$band" <<'PY'
import sys
rate, price, band = (int(x) for x in sys.argv[1:4])
lo = -(-rate * (10_000 - band) // 10_000)  # ceil, as the contract rounds the low edge
hi = rate * (10_000 + band) // 10_000      # floor, as the contract rounds the high edge
dev = (price - rate) * 10_000 / rate
print(f"{'ok' if lo <= price <= hi else 'out'} {dev:+.2f}")
PY
)"
  echo "ℹ price is ${verdict#* } bps from the rate; band is ±$band bps (otc_max_deviation_bps)"
  [[ "${verdict%% *}" == "ok" ]] \
    || die "price is outside the band; the contract would refuse it with InvalidArgument (411)"
}

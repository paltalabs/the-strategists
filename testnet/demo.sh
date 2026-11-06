#!/usr/bin/env bash
# Testnet demo of the RWA yield-sharing strategy behind a DeFindex vault. One subcommand per
# step; every transaction is appended to demo/rwa-yield-sharing/run-log.md with its hash.
#
#   ./demo.sh fund <user|G...> <usdc>       mint test USDC to ts-alice / ts-bob or any address
#   ./demo.sh deposit <user> <usdc>         user deposits into the vault (stays idle in the vault)
#   ./demo.sh rebalance <usdc>              vault manager moves idle USDC into the strategy
#   ./demo.sh invest <usdc>                 strategy keeper buys TBOND on the pool with strategy idle USDC
#   ./demo.sh nav <bps>                     raise the TBOND NAV by <bps> on the oracle (yield accrual)
#   ./demo.sh arb                           LP buys TBOND on the pool until its price is back near NAV
#   ./demo.sh fees                          vault reports gains, locks the vault fee on them, distributes it
#   ./demo.sh withdraw <user> <shares|all>  user withdraws from the vault
#   ./demo.sh claim <user>                  user claims test TUSDC and TBOND from the faucet
#   ./demo.sh refill                        admin tops the faucet up (1M TUSDC, 100k TBOND)
#   ./demo.sh status                        prints vault, strategy and participant balances
source "$(dirname "${BASH_SOURCE[0]}")/_env.sh"

USDC="$(state_get usdc)"; RWA="$(state_get rwa)"; POOL="$(state_get pool)"
STRATEGY="$(state_get strategy)"; VAULT="$(state_get vault)"
[[ -n "$VAULT" ]] || die "run setup.sh first"

amt() { python3 -I -c 'import sys,decimal; print(int(decimal.Decimal(sys.argv[1])*10**7))' "$1"; }
fmt() { python3 -I -c 'import sys,decimal; print(f"{decimal.Decimal(sys.argv[1].strip(chr(34)))/10**7:,.7f}")' "$1"; }

# tx <description> <contract> <signer> -- <fn> [args...]   submits, records hash and result
tx() {
  local desc="$1" id="$2" src="$3"; shift 3
  local err out hash
  err="$(mktemp)"
  out="$(retry stellar contract invoke --id "$id" --source-account "$src" --network "$NET" "$@" 2>"$err")" || { cat "$err" >&2; die "$desc failed"; }
  hash="$(grep -oE '[0-9a-f]{64}' "$err" | head -1 || true)"
  rm -f "$err"
  [[ -s "$LOG" ]] || record "| Time (UTC) | Step | Result | Transaction |"$'\n'"|---|---|---|---|"
  record "| $(date -u '+%Y-%m-%d %H:%M') | $desc | \`$(echo "$out" | tr -d '\n' | cut -c1-80)\` | [${hash:0:8}…](https://stellar.expert/explorer/testnet/tx/$hash) |"
  echo "$desc → $out (tx $hash)"
}

status() {
  echo "== Strategy pricing";   view "$STRATEGY" -- get_pricing_state
  echo "== Oracle NAV (14 dp)"; state_get nav
  echo "== Strategy balance held for the vault (USDC)"; fmt "$(view "$STRATEGY" -- balance --from "$VAULT")"
  echo "== Strategy idle USDC / TBOND held"; fmt "$(view "$USDC" -- balance --id "$STRATEGY")"; fmt "$(view "$RWA" -- balance --id "$STRATEGY")"
  echo "== Vault idle USDC"; fmt "$(view "$USDC" -- balance --id "$VAULT")"
  echo "== Vault total managed funds"; view "$VAULT" -- fetch_total_managed_funds
  echo "== Vault shares total"; fmt "$(view "$VAULT" -- total_supply)"
  for u in "${USERS[@]}"; do
    echo "== $u: shares $(fmt "$(view "$VAULT" -- balance --id "$(addr "$u")")"), wallet USDC $(fmt "$(view "$USDC" -- balance --id "$(addr "$u")")")"
  done
  echo "== Partner (vault fee receiver) USDC: $(fmt "$(view "$USDC" -- balance --id "$(state_get partner)")")"
  echo "== DeFindex protocol receiver USDC: $(fmt "$(view "$USDC" -- balance --id "$(state_get defindex_receiver)")")"
}

case "${1:-}" in
  fund)
    to="$2"; [[ "$to" == G* ]] || to="$(addr "$2")"
    tx "Mint $3 TUSDC to $2" "$USDC" "$ADMIN_ID" -- mint --to "$to" --amount "$(amt "$3")" ;;
  deposit)
    a="$(amt "$3")"
    tx "$2 deposits $3 TUSDC into the vault" "$VAULT" "$2" -- deposit --amounts_desired "[\"$a\"]" --amounts_min "[\"$a\"]" --from "$(addr "$2")" --invest false ;;
  rebalance)
    tx "Vault manager invests $2 TUSDC into the strategy" "$VAULT" "$ADMIN_ID" -- rebalance --caller "$(state_get admin)" \
      --instructions "[{\"Invest\":[\"$STRATEGY\",\"$(amt "$2")\"]}]" ;;
  invest)
    tx "Keeper buys TBOND with $2 TUSDC on the pool" "$STRATEGY" "$KEEPER_ID" -- invest --usdc_amount "$(amt "$2")" ;;
  nav)
    new="$(python3 -I -c 'import sys; print(int(sys.argv[1])*(10000+int(sys.argv[2]))//10000)' "$(state_get nav)" "$2")"
    "$HERE/push_nav.sh" "$new"
    record "| $(date -u '+%Y-%m-%d %H:%M') | NAV +$2 bps on the oracle | \`$new\` | oracle push |"
    echo "NAV now $new" ;;
  arb)
    # Buy TBOND with USDC on the pool so its price follows NAV (rwa is token0: USDC in = one_for_zero).
    zfo=true; [[ "$(state_get rwa_is_token0)" == true ]] && zfo=false
    hints="$(view "$POOL" -- get_oracle_hints)"
    lim="$([[ "$zfo" == true ]] && echo 4295128740 || echo 1461446703485210103287273052203988822378723970341)"
    tx "LP arbitrages the pool toward NAV (${2:-1000} TUSDC)" "$POOL" "$LP_ID" -- swap --sender "$(addr "$LP_ID")" --recipient "$(addr "$LP_ID")" \
      --zero_for_one "$zfo" --amount_specified "$(amt "${2:-1000}")" --sqrt_price_limit_x96 "$lim" --hints "$hints" ;;
  fees)
    tx "Vault reports strategy gains from the strategy balance" "$VAULT" "$ADMIN_ID" -- report
    tx "Vault manager locks the vault fee on strategy gains" "$VAULT" "$ADMIN_ID" -- lock_fees
    tx "Fees distributed: partner and protocol receiver" "$VAULT" "$ADMIN_ID" -- distribute_fees --caller "$(state_get admin)" ;;
  withdraw)
    u="$(addr "$2")"
    s="$3"; [[ "$s" == all ]] && s="$(view "$VAULT" -- balance --id "$u" | tr -d '"')" || s="$(amt "$s")"
    tx "$2 withdraws $(fmt "$s") shares" "$VAULT" "$2" -- withdraw --withdraw_shares "$s" --min_amounts_out '["0"]' --from "$u" ;;
  claim)
    tx "$2 claims test tokens from the faucet" "$(state_get faucet)" "$2" -- claim --to "$(addr "$2")" ;;
  refill)
    tx "Faucet refilled with TUSDC" "$USDC" "$ADMIN_ID" -- mint --to "$(state_get faucet)" --amount "$(amt 1000000)"
    tx "Faucet refilled with TBOND" "$RWA" "$ADMIN_ID" -- mint --to "$(state_get faucet)" --amount "$(amt 100000)" ;;
  status) status ;;
  *) sed -n 2,16p "$0"; exit 1 ;;
esac

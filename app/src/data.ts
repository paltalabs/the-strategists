// Loaders for the three cards. Each returns plain decoded values; formatting is the UI's job.
// Shapes mirror the contracts' #[contracttype] structs as the SDK decodes them
// (snake_case fields, i128/u64 -> bigint, u32 -> number, unions -> { tag, values }).

import { scValToNative } from '@stellar/stellar-sdk';
import { accountExists, hasTrustline, readInstanceKey, simulate } from './chain';
import type { Contracts } from './config';
import { valueOfRwa } from './format';

interface StrategyAllocation {
  strategy_address: string;
  amount: bigint;
  paused: boolean;
}

interface CurrentAssetInvestmentAllocation {
  asset: string;
  total_amount: bigint;
  idle_amount: bigint;
  invested_amount: bigint;
  strategy_allocations: StrategyAllocation[];
}

export interface Report {
  prev_balance: bigint;
  gains_or_losses: bigint;
  locked_fee: bigint;
}

interface Union {
  tag: string;
  values?: unknown[];
}

interface PricingState {
  rate: bigint;
  raw_rate: bigint;
  mode: Union;
  live_oracles: number;
}

interface StrategyConfig {
  asset: string;
  rwa_asset: string;
  asset_decimals: number;
  rwa_decimals: number;
  venue: Union;
}

interface StrategyParams {
  admin: string;
  keeper: string;
  oracles: unknown[];
  max_swap: bigint;
  max_invested: bigint;
  max_spread_bps: number;
  slippage_bps: number;
  paused: boolean;
  auto_invest: boolean;
  frozen_until: bigint;
  otc_max_deviation_bps: number;
}

export interface TokenInfo {
  symbol: string;
  decimals: number;
  /** SAC `name()`: "CODE:ISSUER" for a classic asset. */
  name: string;
}

export interface VaultState {
  usdc: TokenInfo;
  totalManaged: bigint;
  idle: bigint;
  invested: bigint;
  totalSupply: bigint;
  vaultFeeBps: number;
  protocolRateBps: number;
  feeReceiver: string;
  /** Vault's position in the strategy as reported by the strategy's `balance(vault)`. */
  strategyBalance: bigint;
}

export interface StrategyState {
  usdcSymbol: string;
  mode: string;
  rate: bigint;
  rawRate: bigint;
  liveOracles: number;
  oracleCount: number;
  anchor: bigint | null;
  anchorTs: bigint | null;
  assetDecimals: number;
  rwaDecimals: number;
  rwaSymbol: string;
  venue: string;
  idleUsdc: bigint;
  rwaHeld: bigint;
  rwaValue: bigint;
  maxSwap: bigint;
  maxInvested: bigint;
  maxSpreadBps: number;
  slippageBps: number;
  paused: boolean;
  autoInvest: boolean;
  frozenUntil: bigint;
}

export interface Position {
  accountExists: boolean;
  /** null when the asset is not a classic asset (no trustline concept). */
  trustline: boolean | null;
  usdcBalance: bigint;
  shares: bigint;
}

const tokenCache = new Map<string, Promise<TokenInfo>>();

/** Token metadata never changes, so it is read once per session (a failure is retried). */
export function loadToken(id: string): Promise<TokenInfo> {
  let cached = tokenCache.get(id);
  if (!cached) {
    cached = Promise.all([
      simulate<string>(id, 'symbol'),
      simulate<number>(id, 'decimals'),
      simulate<string>(id, 'name'),
    ]).then(([symbol, decimals, name]) => ({ symbol, decimals, name }));
    cached.catch(() => tokenCache.delete(id));
    tokenCache.set(id, cached);
  }
  return cached;
}

export async function loadVault(c: Contracts): Promise<VaultState> {
  const [usdc, funds, fees, feeReceiver, totalSupply, strategyBalance] = await Promise.all([
    loadToken(c.usdc),
    simulate<CurrentAssetInvestmentAllocation[]>(c.vault, 'fetch_total_managed_funds'),
    simulate<[number, number]>(c.vault, 'get_fees'),
    simulate<string>(c.vault, 'get_fee_receiver'),
    simulate<bigint>(c.vault, 'total_supply'),
    simulate<bigint>(c.strategy, 'balance', { from: c.vault }),
  ]);
  // Single-asset vault: the one entry is USDC.
  const usdcFunds = funds.find((f) => f.asset === c.usdc) ?? funds[0];
  return {
    usdc,
    totalManaged: usdcFunds?.total_amount ?? 0n,
    idle: usdcFunds?.idle_amount ?? 0n,
    invested: usdcFunds?.invested_amount ?? 0n,
    totalSupply,
    vaultFeeBps: fees[0],
    protocolRateBps: fees[1],
    feeReceiver,
    strategyBalance,
  };
}

/** Simulates the vault's `report()` (it writes reports on-chain, so it is never sent). */
export function simulateReport(c: Contracts): Promise<Report[]> {
  return simulate<Report[]>(c.vault, 'report');
}

async function readOptionalI128(contractId: string, key: string): Promise<bigint | null> {
  const val = await readInstanceKey(contractId, key);
  return val ? BigInt(scValToNative(val) as bigint | number) : null;
}

export async function loadStrategy(c: Contracts): Promise<StrategyState> {
  const [usdc, rwa, pricing, config, params, idleUsdc, rwaHeld, anchor, anchorTs] = await Promise.all([
    loadToken(c.usdc),
    loadToken(c.rwa),
    simulate<PricingState>(c.strategy, 'get_pricing_state'),
    simulate<StrategyConfig>(c.strategy, 'get_config'),
    simulate<StrategyParams>(c.strategy, 'get_params'),
    simulate<bigint>(c.usdc, 'balance', { id: c.strategy }),
    simulate<bigint>(c.rwa, 'balance', { id: c.strategy }),
    readOptionalI128(c.strategy, 'LastValidPrice'),
    readOptionalI128(c.strategy, 'AnchorTs'),
  ]);
  return {
    usdcSymbol: usdc.symbol,
    rwaSymbol: rwa.symbol,
    mode: pricing.mode.tag,
    rate: pricing.rate,
    rawRate: pricing.raw_rate,
    liveOracles: pricing.live_oracles,
    oracleCount: params.oracles.length,
    anchor,
    anchorTs,
    assetDecimals: config.asset_decimals,
    rwaDecimals: config.rwa_decimals,
    venue: config.venue.tag,
    idleUsdc,
    rwaHeld,
    rwaValue: valueOfRwa(rwaHeld, pricing.rate, config.rwa_decimals),
    maxSwap: params.max_swap,
    maxInvested: params.max_invested,
    maxSpreadBps: params.max_spread_bps,
    slippageBps: params.slippage_bps,
    paused: params.paused,
    autoInvest: params.auto_invest,
    frozenUntil: params.frozen_until,
  };
}

/** Splits a SAC name "CODE:ISSUER" into its parts; null for native or a non-SAC token. */
export function classicAsset(name: string): { code: string; issuer: string } | null {
  const [code, issuer] = name.split(':');
  return code && issuer && issuer.startsWith('G') ? { code, issuer } : null;
}

export async function loadPosition(c: Contracts, account: string): Promise<Position> {
  const [usdc, exists] = await Promise.all([loadToken(c.usdc), accountExists(account)]);
  const asset = classicAsset(usdc.name);
  const trustline = !asset || !exists || asset.issuer === account ? null : await hasTrustline(account, asset.code, asset.issuer);
  const [usdcBalance, shares] = await Promise.all([
    // No trustline or no account: it cannot hold the token, so skip the read.
    trustline === false || !exists ? Promise.resolve(0n) : simulate<bigint>(c.usdc, 'balance', { id: account }),
    simulate<bigint>(c.vault, 'balance', { id: account }),
  ]);
  return { accountExists: exists, trustline, usdcBalance, shares };
}

interface PriceData {
  price: bigint;
  timestamp: bigint;
}

export interface Roles {
  vaultManager: string;
  rebalanceManager: string;
  emergencyManager: string;
  feeReceiver: string;
  strategyAdmin: string;
  strategyKeeper: string;
  oracleAdmin: string | null;
  tokenAdmin: string | null;
  oracleDecimals: number;
  oracleResolutionMs: number;
  /** Last NAV pushed to the oracle for the RWA (timestamp in seconds), null if none. */
  oracleNav: PriceData | null;
  /** False once the last push is too old for the oracle to answer `lastprice`. */
  oracleLive: boolean;
}

/**
 * `lastprice` turns null once the last push is a slot too old, but `price` at
 * `last_timestamp` still returns that push, so the admin can re-push the same NAV.
 */
async function loadLastPushedNav(c: Contracts): Promise<{ nav: PriceData | null; live: boolean }> {
  const asset = { tag: 'Stellar', values: [c.rwa] };
  const [last, lastTs] = await Promise.all([
    simulate<PriceData | null>(c.oracle, 'lastprice', { asset }),
    simulate<bigint>(c.oracle, 'last_timestamp'),
  ]);
  if (last) return { nav: last, live: true };
  if (lastTs === 0n) return { nav: null, live: false };
  const stale = await simulate<PriceData | null>(c.oracle, 'price', { asset, timestamp: lastTs });
  return { nav: stale ?? null, live: false };
}

/** Who holds each role the admin tab can act as, plus the oracle's last NAV. */
export async function loadRoles(c: Contracts): Promise<Roles> {
  const [vaultManager, rebalanceManager, emergencyManager, feeReceiver, params, oracleAdmin, oracleDecimals, oracleResolutionMs, oracleNav] =
    await Promise.all([
      simulate<string>(c.vault, 'get_manager'),
      simulate<string>(c.vault, 'get_rebalance_manager'),
      simulate<string>(c.vault, 'get_emergency_manager'),
      simulate<string>(c.vault, 'get_fee_receiver'),
      simulate<StrategyParams>(c.strategy, 'get_params'),
      simulate<string | null>(c.oracle, 'admin'),
      simulate<number>(c.oracle, 'decimals'),
      simulate<number>(c.oracle, 'resolution'),
      loadLastPushedNav(c),
    ]);
  return {
    vaultManager,
    rebalanceManager,
    emergencyManager,
    feeReceiver,
    strategyAdmin: params.admin,
    strategyKeeper: params.keeper,
    oracleAdmin: oracleAdmin ?? null,
    tokenAdmin: c.token_admin,
    oracleDecimals,
    oracleResolutionMs,
    oracleNav: oracleNav.nav,
    oracleLive: oracleNav.live,
  };
}

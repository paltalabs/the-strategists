// Display helpers. All on-chain amounts are bigint in the token's smallest unit.

/** Matches the strategy's SCALAR_12 (constants.rs). */
export const SCALAR_12 = 1_000_000_000_000n;
export const BPS = 10_000n;

export function pow10(n: number): bigint {
  return 10n ** BigInt(n);
}

/** `raw` units with `decimals` decimals as a fixed string, trimmed to `shown` places. */
export function formatUnits(raw: bigint, decimals: number, shown = 2): string {
  const negative = raw < 0n;
  const abs = negative ? -raw : raw;
  const base = pow10(decimals);
  const whole = abs / base;
  const frac = (abs % base).toString().padStart(decimals, '0').slice(0, shown);
  const wholeStr = whole.toLocaleString('en-US');
  const body = shown > 0 ? `${wholeStr}.${frac.padEnd(shown, '0')}` : wholeStr;
  return negative ? `-${body}` : body;
}

/** Parses a user-typed decimal string into raw units. Returns null on bad input. */
export function parseUnits(input: string, decimals: number): bigint | null {
  const trimmed = input.trim();
  if (!/^\d+(\.\d+)?$/.test(trimmed)) return null;
  const [whole, frac = ''] = trimmed.split('.');
  if (frac.length > decimals) return null;
  return BigInt(whole) * pow10(decimals) + BigInt(frac.padEnd(decimals, '0') || '0');
}

/** Basis points as a percentage string, e.g. 2000 -> "20.00%". */
export function formatBps(bps: bigint): string {
  const sign = bps < 0n ? '-' : '';
  const abs = bps < 0n ? -bps : bps;
  return `${sign}${abs / 100n}.${(abs % 100n).toString().padStart(2, '0')}%`;
}

/**
 * The strategy's rate as USD per whole RWA. The contract stores
 * `price * 10^asset_decimals * SCALAR_12` (math.rs `rate_from_price`), so dividing by
 * `10^asset_decimals * SCALAR_12` gives the plain price.
 */
export function formatRate(rate: bigint, assetDecimals: number, shown = 6): string {
  return formatUnits(rate, assetDecimals + 12, shown);
}

/** USDC stroops `qty` raw RWA units are worth at `rate` (math.rs `value_of`, floored). */
export function valueOfRwa(qty: bigint, rate: bigint, rwaDecimals: number): bigint {
  if (rate <= 0n) return 0n;
  return (qty * rate) / (pow10(rwaDecimals) * SCALAR_12);
}

export function shortAddress(address: string): string {
  return address.length > 12 ? `${address.slice(0, 5)}…${address.slice(-5)}` : address;
}

export function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === 'string') return err;
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

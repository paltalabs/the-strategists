// Network constants and the deployed contract addresses.
//
// Addresses come from ../testnet.contracts.json (the-strategists repo root). The file does
// not exist until the contracts are deployed, so it is loaded through import.meta.glob:
// a missing file yields an empty match instead of a build error, and the app shows a
// "not deployed yet" state.

export const NETWORK_PASSPHRASE = 'Test SDF Network ; September 2015';
/** Testnet Soroban RPC. Override at build time with VITE_RPC_URL. */
export const RPC_URL: string = import.meta.env.VITE_RPC_URL || 'https://soroban-rpc.testnet.stellar.gateway.fm';
export const EXPLORER_TX = 'https://stellar.expert/explorer/testnet/tx/';
export const EXPLORER_CONTRACT = 'https://stellar.expert/explorer/testnet/contract/';
export const EXPLORER_ACCOUNT = 'https://stellar.expert/explorer/testnet/account/';
export const REFRESH_MS = 15_000;

export interface Contracts {
  usdc: string;
  rwa: string;
  oracle: string;
  pool: string;
  strategy: string;
  vault: string;
  partner: string;
  defindex_receiver: string;
  /** Admin of the test tokens (mint). Only the deploy script knows it; null if absent. */
  token_admin: string | null;
  /** Test token faucet, null if not deployed. */
  faucet: string | null;
}

const REQUIRED_KEYS: ReadonlyArray<Exclude<keyof Contracts, 'token_admin' | 'faucet'>> = [
  'usdc',
  'rwa',
  'oracle',
  'pool',
  'strategy',
  'vault',
  'partner',
  'defindex_receiver',
];

const matches = import.meta.glob<unknown>('../../testnet.contracts.json', {
  eager: true,
  import: 'default',
});

function parseContracts(raw: unknown): { contracts: Contracts | null; problem: string | null } {
  if (raw === undefined) {
    return { contracts: null, problem: 'testnet.contracts.json not found' };
  }
  if (typeof raw !== 'object' || raw === null) {
    return { contracts: null, problem: 'testnet.contracts.json is not an object' };
  }
  const record = raw as Record<string, unknown>;
  const missing = REQUIRED_KEYS.filter((k) => typeof record[k] !== 'string' || record[k] === '');
  if (missing.length > 0) {
    return { contracts: null, problem: `testnet.contracts.json is missing: ${missing.join(', ')}` };
  }
  const contracts = {
    ...(Object.fromEntries(REQUIRED_KEYS.map((k) => [k, record[k] as string])) as Omit<Contracts, 'token_admin' | 'faucet'>),
    token_admin: typeof record.admin === 'string' ? record.admin : null,
    faucet: typeof record.faucet === 'string' && record.faucet !== '' ? record.faucet : null,
  };
  return { contracts, problem: null };
}

const parsed = parseContracts(Object.values(matches)[0]);

/** The deployed addresses, or null when the file is missing or incomplete. */
export const CONTRACTS: Contracts | null = parsed.contracts;
/** Why CONTRACTS is null, for the "not deployed yet" banner. */
export const CONTRACTS_PROBLEM: string | null = parsed.problem;

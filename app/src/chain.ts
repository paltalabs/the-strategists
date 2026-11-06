// Soroban RPC access: contract clients built from on-chain specs, read-only simulation, and
// signed submission. No generated bindings: contract.Client.from fetches each contract's
// spec from chain (the embedded SAC spec for Stellar Asset Contracts).

import { Asset, Keypair, Operation, TransactionBuilder, contract, rpc, xdr } from '@stellar/stellar-sdk';
import { NETWORK_PASSPHRASE, RPC_URL } from './config';

export const server = new rpc.Server(RPC_URL);
// The SDK adds X-Client-Name / X-Client-Version to every RPC call. Some public RPCs
// (gateway.fm) do not list them in their CORS allowed headers, so the browser's preflight
// fails. Drop them; every client below shares this server.
for (const h of ['X-Client-Name', 'X-Client-Version']) {
  delete (server.httpClient.defaults.headers as Record<string, unknown>)[h];
}

/** Spec fetch (instance + Wasm, two RPC calls) and single simulations. The public testnet
 * RPC can take several seconds per call, so these are generous but finite: a stuck request
 * becomes a visible error instead of an endless "Loading…". */
const SPEC_TIMEOUT_MS = 30_000;
const READ_TIMEOUT_MS = 25_000;

export class TimeoutError extends Error {}

/** Rejects with a labelled error if `promise` has not settled within `ms`. */
export function withTimeout<T>(promise: Promise<T>, ms: number, label: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new TimeoutError(`${label} timed out after ${ms / 1000}s (testnet RPC slow or unreachable)`)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

type AnyClient = contract.Client & Record<string, unknown>;
type Method = (args?: Record<string, unknown>) => Promise<contract.AssembledTransaction<unknown>>;

const clientCache = new Map<string, Promise<AnyClient>>();

/** Read-only client for `contractId`, cached for the session (the spec never changes). */
export function getClient(contractId: string): Promise<AnyClient> {
  let cached = clientCache.get(contractId);
  if (!cached) {
    cached = contract.Client.from({ contractId, networkPassphrase: NETWORK_PASSPHRASE, rpcUrl: RPC_URL, server }) as Promise<AnyClient>;
    // Do not cache a failure (e.g. contract not deployed yet): retry on the next refresh.
    cached.catch(() => clientCache.delete(contractId));
    clientCache.set(contractId, cached);
  }
  return cached;
}

function methodOf(client: AnyClient, method: string): Method {
  const fn = client[method];
  if (typeof fn !== 'function') {
    throw new Error(`Contract has no method "${method}"`);
  }
  return fn as Method;
}

/** The `Error(Contract, #N)` part of a failed simulation, the most useful bit for a human. */
function simulationErrorText(tx: contract.AssembledTransaction<unknown>): string | null {
  const sim = tx.simulation;
  if (!sim || !rpc.Api.isSimulationError(sim)) return null;
  return sim.error.match(/Error\([A-Za-z]+, #?\w+\)/)?.[0] ?? sim.error.split('\n')[0];
}

/**
 * Unwraps a contract `Result<T, E>` (Ok/Err from the SDK); plain values pass through.
 * The SDK's Err carries the error's doc text, often empty, so the code from the
 * simulation is added.
 */
function unwrapResult(tx: contract.AssembledTransaction<unknown>): unknown {
  const value = tx.result;
  if (typeof value === 'object' && value !== null && 'isOk' in value && 'unwrap' in value) {
    const result = value as contract.Result<unknown>;
    if (!result.isOk()) {
      const err = result.unwrapErr() as { message?: string };
      const parts = [err.message, simulationErrorText(tx)].filter((p): p is string => !!p);
      throw new Error(parts.length > 0 ? `Contract returned an error: ${parts.join(' ')}` : 'Contract returned an error');
    }
    return result.unwrap();
  }
  return value;
}

/**
 * Simulates `method` and returns its decoded return value. Nothing is signed or sent, so
 * this is also how the app previews state-changing calls such as the vault's `report()`.
 */
export async function simulate<T>(contractId: string, method: string, args?: Record<string, unknown>): Promise<T> {
  // A pending spec fetch stays cached, so a timeout here does not restart it next refresh.
  const client = await withTimeout(getClient(contractId), SPEC_TIMEOUT_MS, `Loading contract spec for ${method}`);
  const tx = await withTimeout(methodOf(client, method).call(client, args), READ_TIMEOUT_MS, `Reading ${method}`);
  return unwrapResult(tx) as T;
}

export type Signer = (txXdr: string) => Promise<string>;

/** Sends a signed transaction envelope and waits until the network reports it final. */
async function sendSigned(signedXdr: string): Promise<string> {
  const tx = TransactionBuilder.fromXDR(signedXdr, NETWORK_PASSPHRASE);
  const sent = await server.sendTransaction(tx);
  if (sent.status === 'ERROR' || sent.status === 'TRY_AGAIN_LATER') {
    throw new Error(`Submission rejected (${sent.status}), tx ${sent.hash}`);
  }
  const final = await server.pollTransaction(sent.hash, { attempts: 30 });
  if (final.status !== rpc.Api.GetTransactionStatus.SUCCESS) {
    throw new Error(`Transaction ${sent.hash} ended as ${final.status}`);
  }
  return sent.hash;
}

/**
 * Builds `method` with `source` as the transaction source, simulates it (which fills in
 * the footprint, resources and fee), has the wallet sign it, submits it and returns the hash.
 */
export async function invoke(
  contractId: string,
  method: string,
  args: Record<string, unknown>,
  source: string,
  sign: Signer,
): Promise<string> {
  const readClient = await getClient(contractId);
  const client = new contract.Client(readClient.spec, { ...readClient.options, publicKey: source }) as AnyClient;
  const tx = await methodOf(client, method).call(client, args);
  // A failed simulation or an Err result means the call would revert: stop before the wallet.
  unwrapResult(tx);
  const signed = await sign(tx.toXdr());
  return sendSigned(signed);
}

/** True when `account` holds a trustline to the classic asset `code:issuer`. */
export async function hasTrustline(account: string, code: string, issuer: string): Promise<boolean> {
  const key = xdr.LedgerKey.trustline(
    new xdr.LedgerKeyTrustLine({
      accountId: Keypair.fromPublicKey(account).xdrAccountId(),
      asset: new Asset(code, issuer).toTrustLineXdrObject(),
    }),
  );
  const res = await withTimeout(server.getLedgerEntries(key), READ_TIMEOUT_MS, 'Reading trustline');
  return res.entries.length > 0;
}

/** True when `account` exists on the ledger (an unfunded account cannot sign anything). */
export async function accountExists(account: string): Promise<boolean> {
  try {
    await withTimeout(server.getAccount(account), READ_TIMEOUT_MS, 'Reading account');
    return true;
  } catch (err) {
    // A slow RPC is not proof the account is missing.
    if (err instanceof TimeoutError) throw err;
    return false;
  }
}

/** Adds a trustline to `code:issuer` for `account`, signed by the wallet. Returns the hash. */
export async function addTrustline(account: string, code: string, issuer: string, sign: Signer): Promise<string> {
  const source = await server.getAccount(account);
  const tx = new TransactionBuilder(source, { fee: '1000', networkPassphrase: NETWORK_PASSPHRASE })
    .addOperation(Operation.changeTrust({ asset: new Asset(code, issuer) }))
    .setTimeout(180)
    .build();
  const signed = await sign(tx.toXDR());
  return sendSigned(signed);
}

/**
 * Reads one unit-variant key (e.g. `LastValidPrice`) from a contract's instance storage.
 * `#[contracttype]` encodes a unit enum variant as `Vec[Symbol(name)]`. Returns null when
 * the key is unset.
 */
export async function readInstanceKey(contractId: string, variant: string): Promise<xdr.ScVal | null> {
  const instance = await withTimeout(server.getContractInstance(contractId), READ_TIMEOUT_MS, `Reading ${variant}`);
  const wanted = xdr.ScVal.scvVec([xdr.ScVal.scvSymbol(variant)]).toXdr('base64');
  const hit = (instance.storage ?? []).find((entry) => entry.key.toXdr('base64') === wanted);
  return hit ? hit.val : null;
}

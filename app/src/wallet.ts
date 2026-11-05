// Wallet connection through Stellar Wallets Kit, pinned to TESTNET.

import { StellarWalletsKit } from '@creit.tech/stellar-wallets-kit/sdk';
import { defaultModules } from '@creit.tech/stellar-wallets-kit/modules/utils';
import { Networks } from '@creit.tech/stellar-wallets-kit/types';
import { NETWORK_PASSPHRASE } from './config';
import type { Signer } from './chain';

let initialized = false;

function ensureInit(): void {
  if (initialized) return;
  // defaultModules() includes Freighter plus every wallet that needs no extra setup.
  StellarWalletsKit.init({ modules: defaultModules(), network: Networks.TESTNET });
  initialized = true;
}

/** Opens the wallet picker and returns the connected address. */
export async function connectWallet(): Promise<string> {
  ensureInit();
  const { address } = await StellarWalletsKit.authModal();
  return address;
}

export async function disconnectWallet(): Promise<void> {
  ensureInit();
  await StellarWalletsKit.disconnect();
}

/** A signer bound to `address` on testnet, for chain.invoke / chain.addTrustline. */
export function signerFor(address: string): Signer {
  ensureInit();
  return async (txXdr: string): Promise<string> => {
    const { signedTxXdr } = await StellarWalletsKit.signTransaction(txXdr, {
      networkPassphrase: NETWORK_PASSPHRASE,
      address,
    });
    return signedTxXdr;
  };
}

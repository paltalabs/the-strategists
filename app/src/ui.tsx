import type { ReactNode } from 'react';
import { EXPLORER_ACCOUNT, EXPLORER_CONTRACT, EXPLORER_TX } from './config';
import { shortAddress } from './format';

export function Card(props: { title: string; error?: string | null; children: ReactNode; aside?: ReactNode }) {
  return (
    <section className="card">
      <header className="card-head">
        <h2>{props.title}</h2>
        {props.aside}
      </header>
      {props.error && <p className="error">{props.error}</p>}
      {props.children}
    </section>
  );
}

export function Row(props: { label: ReactNode; children: ReactNode; hint?: string }) {
  return (
    <div className="row" title={props.hint}>
      <span className="label">{props.label}</span>
      <span className="value">{props.children}</span>
    </div>
  );
}

/** Address with a link to the explorer; C... is a contract, G... an account. */
export function AddressLink(props: { address: string }) {
  const base = props.address.startsWith('C') ? EXPLORER_CONTRACT : EXPLORER_ACCOUNT;
  return (
    <a className="mono" href={`${base}${props.address}`} target="_blank" rel="noreferrer" title={props.address}>
      {shortAddress(props.address)}
    </a>
  );
}

export interface TxStatus {
  kind: 'pending' | 'ok' | 'error';
  label: string;
  hash?: string;
  message?: string;
}

export function TxNotice(props: { status: TxStatus | null }) {
  const s = props.status;
  if (!s) return null;
  return (
    <div className={`notice notice-${s.kind}`}>
      <strong>{s.label}</strong>{' '}
      {s.kind === 'pending' && <span>waiting for wallet and network…</span>}
      {s.hash && (
        <a className="mono" href={`${EXPLORER_TX}${s.hash}`} target="_blank" rel="noreferrer">
          {s.hash.slice(0, 10)}…{s.hash.slice(-6)}
        </a>
      )}
      {s.message && <span className="notice-msg">{s.message}</span>}
    </div>
  );
}

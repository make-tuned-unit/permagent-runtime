import { useEffect } from 'react';
import type { IdentityState } from '../../../services/identity';
import { Button } from '../../common/Button';
import { Tooltip } from '../../common/Tooltip';
import { useTheme } from '../../../styles/useTheme';
import { useIdentityStore } from '../../../stores/identityStore';

/**
 * The sovereign's digital passport, on the forum HUD.
 *
 * The identity surface existed only on the legacy `WorldView` (`HenryHUD` →
 * `HenryIdentityTab`), so moving the World over to the Solar Forum took the
 * passport off the screen — Jesse's words: "the HUD with the identity card and
 * specifically the Chitin ID and sovereign status of my agent is no longer
 * there, that is my sovereigns digital passport."
 *
 * This is the compact, always-on forum form of it: who the agent is, its Chitin
 * DID, its passport token, and whether that identity is currently verified.
 *
 * It reads the SAME `identityStore` the legacy tab reads — chitin.id merged
 * with the on-chain owner — so nothing here is decorative or invented. When the
 * store has never fetched, or the network is down, the card says so in those
 * words rather than showing a confident-looking placeholder: a passport that
 * lies about its own status is worse than one that admits it is offline.
 */
export function ForumPassport({ onOpen }: { onOpen?: () => void }) {
  const { colors } = useTheme();
  // `data` is null until the first successful fetch (and stays null when the
  // machine has never been online), so every read below goes through an empty
  // object rather than optional-chaining each field at the point of use.
  const stored = useIdentityStore((s) => s.data);
  const data: Partial<IdentityState> = stored ?? {};
  const loading = useIdentityStore((s) => s.loading);
  const connectivity = useIdentityStore((s) => s.connectivity);
  const lastSuccessfulFetch = useIdentityStore((s) => s.lastSuccessfulFetch);
  const startPolling = useIdentityStore((s) => s.startPolling);
  const stopPolling = useIdentityStore((s) => s.stopPolling);

  // Ref-counted shared poll: the store owns the interval, so mounting this
  // alongside the legacy HUD cannot double-poll chitin.id or the RPC.
  useEffect(() => {
    startPolling();
    return () => stopPolling();
  }, [startPolling, stopPolling]);

  const did = data.did?.trim() ?? '';
  // `did:chitin:henry-malcolm` → `henry-malcolm`, which is the Chitin ID
  // itself; the full DID stays in the title attribute for copy/paste.
  const chitinId = did.startsWith('did:chitin:') ? did.slice('did:chitin:'.length) : did;
  const verified = data.soulValid === true;
  const everFetched = lastSuccessfulFetch !== null;

  // The pill states ONE thing — whether the soul is currently valid — because
  // mixing the record's own `status` string in here produced a card that read
  // "UNVERIFIED" next to "Soul: sealed · valid". The record's wording gets its
  // own row below, where it cannot contradict the seal.
  const status = !everFetched
    ? { label: loading ? 'READING PASSPORT' : 'NOT YET READ', tone: 'pending' as const }
    : connectivity === 'offline'
      ? { label: 'LAST KNOWN · OFFLINE', tone: 'stale' as const }
      : verified
        ? { label: 'SOVEREIGN · VERIFIED', tone: 'ok' as const }
        : { label: 'NOT VERIFIED', tone: 'warn' as const };
  const recordStatus = data.status?.trim();

  return (
    <div
      className="forum-passport"
      data-testid="forum-passport"
      data-tone={status.tone}
      role="group"
      aria-label="Sovereign digital passport"
    >
      <div className="forum-passport-head">
        <span className="eyebrow">SOVEREIGN PASSPORT</span>
        <span className="forum-passport-status" data-testid="forum-passport-status">{status.label}</span>
      </div>
      <div className="forum-passport-name">{data.name?.trim() || 'Henry'}</div>
      <dl className="forum-passport-rows">
        <div>
          <dt>Chitin ID</dt>
          <Tooltip content={did || 'no decentralised identifier on record'}>
            <dd data-testid="forum-passport-chitin">
              {chitinId || (everFetched ? 'none on record' : '—')}
            </dd>
          </Tooltip>
        </div>
        <div>
          <dt>Passport</dt>
          <dd>{data.passportId ? `#${data.passportId}` : everFetched ? 'none minted' : '—'}</dd>
        </div>
        <div>
          <dt>Soul</dt>
          <dd>{everFetched ? (verified ? 'sealed · valid' : 'not valid') : '—'}</dd>
        </div>
        {recordStatus && (
          <div>
            <dt>On record</dt>
            <dd>{recordStatus}</dd>
          </div>
        )}
      </dl>
      {onOpen && (
        <Button
          colors={colors}
          variant="bare"
          className="forum-passport-more"
          flashSuccess={false}
          onClick={onOpen}
        >
          Full passport
        </Button>
      )}
    </div>
  );
}

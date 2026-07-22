/**
 * Governance → Sovereignty. The data boundary toggle + the "pull the cable, see
 * everything that tried to leave" egress audit log. Reads the REAL
 * `GET /api/security/sovereignty` and `GET /api/security/egress-log`, and flips
 * the boundary through `POST /api/security/sovereignty`. Append-only log — this
 * view never mutates it.
 */

import { useCallback, useEffect, useState } from 'react';
import { api, type EgressLogEntry, type SovereigntyStatus } from '../../lib/api';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Card, SectionLabel } from './atoms';
import { timeAgo } from './format';

const COLS = '150px 1fr 90px 80px';

export function SovereigntyPanel() {
  const { colors } = useTheme();
  const [status, setStatus] = useState<SovereigntyStatus | null>(null);
  const [log, setLog] = useState<EgressLogEntry[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadLog = useCallback(() => {
    api.getEgressLog(100)
      .then(setLog)
      .catch(() => setLog([]));
  }, []);

  useEffect(() => {
    let active = true;
    api.getSovereignty()
      .then(s => { if (active) { setStatus(s); setError(null); } })
      .catch(() => { if (active) setError('Could not load sovereignty status.'); });
    loadLog();
    return () => { active = false; };
  }, [loadLog]);

  const toggle = useCallback(async () => {
    if (!status) return;
    setBusy(true);
    try {
      const next = await api.setSovereignty({ enabled: !status.enabled });
      setStatus(next);
      // A flip changes what the guard does next; refresh the log so a newly
      // blocked call shows up.
      loadLog();
    } catch {
      setError('Could not change the data boundary.');
    } finally {
      setBusy(false);
    }
  }, [status, loadLog]);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <Card>
        <SectionLabel>Data boundary — enforced locally, not by a cloud admin</SectionLabel>
        <div style={{ display: 'flex', alignItems: 'center', gap: 16, marginTop: 12, flexWrap: 'wrap' }}>
          <div style={{ flex: 1, minWidth: 240 }}>
            <div style={{ fontFamily: font.display, fontSize: 18, fontWeight: 600 }}>
              {status === null ? '—' : status.enabled ? 'Sovereign mode ON' : 'Sovereign mode OFF'}
            </div>
            <div style={{ fontSize: 12, color: colors.textMuted, marginTop: 4 }}>
              {status?.enabled
                ? 'All cloud inference is blocked before any data leaves this machine. Only local models run.'
                : 'Cloud inference is allowed. Every cloud call is still recorded in the audit log below.'}
              {status && !status.localProviderAvailable && (
                <span style={{ color: colors.warning }}> No local provider is registered — sovereign work has nowhere to run yet.</span>
              )}
            </div>
          </div>
          <button
            onClick={toggle}
            disabled={busy || status === null}
            style={{
              height: 32, padding: '0 18px', borderRadius: 8,
              background: status?.enabled ? 'transparent' : colors.cyan,
              border: status?.enabled ? `1px solid ${colors.borderHi}` : 'none',
              color: status?.enabled ? colors.cyan : colors.textOnAccent,
              cursor: busy ? 'default' : 'pointer', fontFamily: font.body, fontSize: 12, fontWeight: 600,
            }}
          >
            {busy ? '…' : status?.enabled ? 'Allow cloud again' : 'Pull the cable'}
          </button>
        </div>
        {error && <div style={{ fontSize: 12, color: colors.danger, marginTop: 10 }}>{error}</div>}
      </Card>

      <Card>
        <SectionLabel>Egress audit — every cloud call, allowed or blocked</SectionLabel>
        {log === null ? (
          <div style={{ color: colors.textDim, fontSize: 13, marginTop: 10 }}>Loading audit log…</div>
        ) : log.length === 0 ? (
          <div style={{ color: colors.textMuted, fontSize: 13, marginTop: 10 }}>
            Nothing has left this machine yet. Every cloud call will be recorded here.
          </div>
        ) : (
          <div style={{ marginTop: 12, overflowX: 'auto' }}>
            <div style={{ minWidth: 460 }}>
              <div style={{ display: 'grid', gridTemplateColumns: COLS, gap: 12, padding: '0 10px 8px', fontSize: 10, fontWeight: 600, letterSpacing: '0.08em', textTransform: 'uppercase', color: colors.textDim }}>
                <div>When</div><div>Provider · Model</div><div>Kind</div><div>Result</div>
              </div>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                {log.map(e => (
                  <div key={e.id} style={{ display: 'grid', gridTemplateColumns: COLS, gap: 12, alignItems: 'center', padding: '8px 10px', borderRadius: 8, background: colors.bgDeeper, border: `1px solid ${colors.border}` }}>
                    <div style={{ fontSize: 12, color: colors.textMuted, fontFamily: font.mono }} title={e.ts}>{timeAgo(e.ts) || e.ts}</div>
                    <div style={{ fontSize: 12, color: colors.text, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={`${e.provider} · ${e.model}`}>
                      <span style={{ color: colors.textMuted }}>{e.provider}</span> · {e.model}
                    </div>
                    <div style={{ fontSize: 12, color: colors.textMuted }}>{e.kind}</div>
                    <div>
                      <span style={{
                        fontSize: 10, fontWeight: 700, letterSpacing: '0.04em', textTransform: 'uppercase',
                        padding: '2px 8px', borderRadius: 999, border: `1px solid ${colors.border}`,
                        color: e.blocked ? colors.warning : colors.success,
                      }}>
                        {e.blocked ? 'blocked' : 'allowed'}
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}
      </Card>
    </div>
  );
}

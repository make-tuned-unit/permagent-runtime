/**
 * Governance → Models. Surfaces the active primary model + goose-mode and the
 * worker roster (per-role engine + live availability), reading the REAL
 * `GET /config` and `GET /api/agent/workers`. Read-only here: changing the
 * model lives in Settings, so a button routes there rather than duplicating the
 * provider editor (no dead controls).
 */

import { useEffect, useState } from 'react';
import { api, type WorkerInfo } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Card, StatRow, SectionLabel } from './atoms';

export function ModelsPanel() {
  const { colors } = useTheme();
  const setActivePanel = useCommandCenter(s => s.setActivePanel);
  const [model, setModel] = useState<string | null>(null);
  const [provider, setProvider] = useState<string | null>(null);
  const [mode, setMode] = useState<string | null>(null);
  const [workers, setWorkers] = useState<WorkerInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    Promise.all([api.getConfig(), api.getWorkers()])
      .then(([cfg, ws]) => {
        if (!active) return;
        const map = ((cfg as Record<string, unknown>)['config'] ?? cfg) as Record<string, unknown>;
        setModel((map['GOOSE_MODEL'] as string) ?? null);
        setProvider((map['GOOSE_PROVIDER'] as string) ?? null);
        setMode((cfg.effective_goose_mode as string) ?? null);
        setWorkers(Object.values(ws));
        setError(null);
      })
      .catch(() => { if (active) { setWorkers([]); setError('Could not load model configuration.'); } });
    return () => { active = false; };
  }, []);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <Card>
        <SectionLabel>Primary model — what you run by default</SectionLabel>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 2, marginTop: 10 }}>
          <div style={{ fontFamily: font.display, fontSize: 22, fontWeight: 600, letterSpacing: '-0.01em' }}>
            {model ?? '—'}
          </div>
          <div style={{ fontSize: 12, color: colors.textMuted, fontFamily: font.mono }}>
            {provider ? `provider: ${provider}` : 'provider: default'}{mode ? ` · mode: ${mode}` : ''}
          </div>
        </div>
        <button
          onClick={() => setActivePanel('settings')}
          style={{
            marginTop: 14, height: 30, padding: '0 14px', borderRadius: 8,
            background: 'transparent', border: `1px solid ${colors.borderHi}`,
            color: colors.cyan, cursor: 'pointer', fontFamily: font.body, fontSize: 12, fontWeight: 600,
            alignSelf: 'flex-start',
          }}
        >
          Change model in Settings →
        </button>
      </Card>

      <Card>
        <SectionLabel>Worker roster — the models each role can dispatch to</SectionLabel>
        {workers === null ? (
          <div style={{ color: colors.textDim, fontSize: 13, marginTop: 10 }}>Loading roster…</div>
        ) : workers.length === 0 ? (
          <div style={{ color: colors.textMuted, fontSize: 13, marginTop: 10 }}>
            {error ?? 'No workers configured. The primary model handles every role.'}
          </div>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8, marginTop: 12 }}>
            {workers.map(w => (
              <StatRow
                key={w.key}
                left={w.display_name}
                sub={`${w.role} · ${w.engine}`}
                right={
                  <span style={{
                    fontSize: 11, fontWeight: 600, padding: '2px 8px', borderRadius: 999,
                    border: `1px solid ${colors.border}`,
                    color: w.available ? colors.success : colors.textDim,
                  }} title={w.reason ?? undefined}>
                    {w.available ? 'available' : 'unavailable'}
                  </span>
                }
              />
            ))}
          </div>
        )}
      </Card>
    </div>
  );
}

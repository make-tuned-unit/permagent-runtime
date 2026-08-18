/**
 * Settings → Models → Routing: the per-role model map, visible and overridable.
 *
 * The cost-router routes each workflow role (orchestrate / edit / mechanical /
 * review / local) to the model best for the job, local or cloud, as cheaply as
 * it can. Until this section that map had no surface in the app — only the CLI
 * (`permagent packs …`). This renders GET /api/cost-router/roles and writes
 * PUT/DELETE /api/cost-router/roles/{role}: the same `PERMAGENT_ROLE_*` keys the
 * CLI writes, so both surfaces agree.
 *
 * Effective model per row (the badge says which):
 *   configured  — the user hand-set it; always wins.
 *   derived     — the router's best-fit over the models actually available.
 *   session model (no fit) — neither; the role runs on the session model.
 */
import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { api, type RoutingModel, type RoutingRoleRow, type RoutingRolesResponse } from '../../lib/api';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Section, TextInput } from './atoms';

type Colors = ReturnType<typeof useTheme>['colors'];

export type EffectiveKind = 'configured' | 'derived' | 'session';

/** The model a role actually runs on, and why. Pure so it is testable. */
export function effectiveModel(row: RoutingRoleRow): { kind: EffectiveKind; model: RoutingModel | null } {
  if (row.configured) return { kind: 'configured', model: row.configured };
  if (row.recommended) return { kind: 'derived', model: row.recommended };
  return { kind: 'session', model: null };
}

const BADGE_LABEL: Record<EffectiveKind, string> = {
  configured: 'configured',
  derived: 'derived',
  session: 'session model (no fit)',
};

const CONFIDENCE_LABEL: Record<NonNullable<RoutingRoleRow['confidence']>, string | null> = {
  exact: null,
  alias: null,
  family_estimate: 'family estimate',
};

function Badge({ kind, colors }: { kind: EffectiveKind; colors: Colors }) {
  const color = kind === 'configured' ? colors.cyan : kind === 'derived' ? colors.success : colors.textDim;
  return (
    <span
      data-testid={`routing-badge-${kind}`}
      style={{
        fontSize: 10, fontWeight: 600, padding: '2px 8px', borderRadius: 999,
        border: `1px solid ${colors.border}`, color, whiteSpace: 'nowrap',
      }}
    >
      {BADGE_LABEL[kind]}
    </span>
  );
}

function RoleRowView({ row, onSaved, colors }: {
  row: RoutingRoleRow;
  onSaved: (row: RoutingRoleRow) => void;
  colors: Colors;
}) {
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Prefill: the hand-set value if any, else the recommendation — so "Override"
  // starts from what the role would run on today.
  const seed = row.configured ?? row.recommended;
  const [provider, setProvider] = useState(seed?.provider ?? '');
  const [model, setModel] = useState(seed?.model ?? '');

  const openEditor = () => {
    const s = row.configured ?? row.recommended;
    setProvider(s?.provider ?? '');
    setModel(s?.model ?? '');
    setError(null);
    setEditing(true);
  };

  const save = async () => {
    const p = provider.trim();
    const m = model.trim();
    if (!p || !m) { setError('Provider and model are both required.'); return; }
    setBusy(true);
    setError(null);
    try {
      const updated = await api.setRoutingRole(row.role, { provider: p, model: m });
      onSaved(updated);
      setEditing(false);
    } catch (e) {
      setError(`Couldn't save: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const clear = async () => {
    setBusy(true);
    setError(null);
    try {
      const updated = await api.clearRoutingRole(row.role);
      onSaved(updated);
      setEditing(false);
    } catch (e) {
      setError(`Couldn't clear: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const eff = effectiveModel(row);
  const confidenceNote = row.confidence ? CONFIDENCE_LABEL[row.confidence] : null;
  const smallBtn: CSSProperties = {
    height: 26, padding: '0 10px', borderRadius: 6, background: 'transparent',
    border: `1px solid ${colors.border}`, color: colors.text, cursor: busy ? 'default' : 'pointer',
    fontFamily: font.body, fontSize: 11, fontWeight: 500, opacity: busy ? 0.6 : 1,
  };
  const primaryBtn: CSSProperties = {
    ...smallBtn, border: 'none', background: colors.cyan, color: colors.textOnAccent, fontWeight: 600,
  };

  return (
    <div
      data-testid={`routing-row-${row.role}`}
      style={{
        padding: '12px 14px', borderRadius: radius.md,
        background: colors.bgDeeper, border: `1px solid ${colors.border}`,
        display: 'flex', flexDirection: 'column', gap: 8,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
            <span style={{ fontSize: 13, fontWeight: 600, color: colors.text }}>{row.label}</span>
            <Badge kind={eff.kind} colors={colors} />
            {eff.kind !== 'session' && (
              <span
                data-testid={`routing-fit-${row.role}`}
                title={row.floor_met ? 'Clears the role\'s capability bar' : 'Below the role\'s capability bar — best available, flagged'}
                style={{ fontSize: 10, fontWeight: 600, color: row.floor_met ? colors.success : colors.warning }}
              >
                {row.floor_met ? 'fits' : 'under-fit'}
              </span>
            )}
          </div>
          <div style={{ fontSize: 11, color: colors.textMuted, marginTop: 2, lineHeight: 1.5 }}>{row.description}</div>
          <div style={{ fontSize: 12, fontFamily: font.mono, color: colors.text, marginTop: 6 }}>
            {eff.model ? `${eff.model.provider}/${eff.model.model}` : 'session model'}
            {confidenceNote && eff.kind === 'derived' && (
              <span style={{ color: colors.textDim, marginLeft: 8, fontFamily: font.body, fontSize: 11 }}>({confidenceNote})</span>
            )}
          </div>
          {eff.kind === 'configured' && row.recommended && (
            <div style={{ fontSize: 11, color: colors.textDim, fontFamily: font.mono, marginTop: 2 }}>
              derived would be {row.recommended.provider}/{row.recommended.model}
            </div>
          )}
          {row.warnings.length > 0 && (
            <ul style={{ margin: '6px 0 0', paddingLeft: 16, fontSize: 11, color: colors.warning, lineHeight: 1.5 }}>
              {row.warnings.map((w, i) => <li key={i}>{w}</li>)}
            </ul>
          )}
        </div>
        <div style={{ flexShrink: 0, display: 'flex', gap: 6 }}>
          {!editing && (
            <button data-testid={`routing-override-${row.role}`} style={smallBtn} disabled={busy} onClick={openEditor}>
              Override
            </button>
          )}
          {!editing && row.configured && (
            <button data-testid={`routing-clear-${row.role}`} style={smallBtn} disabled={busy} onClick={clear}>
              Clear
            </button>
          )}
        </div>
      </div>

      {editing && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8, borderTop: `1px solid ${colors.border}`, paddingTop: 10 }}>
          <div style={{ display: 'flex', gap: 8 }}>
            <div style={{ flex: 1 }} data-testid={`routing-provider-${row.role}`}>
              <TextInput value={provider} onChange={setProvider} placeholder="provider (e.g. anthropic, ollama)" mono />
            </div>
            <div style={{ flex: 2 }} data-testid={`routing-model-${row.role}`}>
              <TextInput value={model} onChange={setModel} placeholder="model id" mono />
            </div>
          </div>
          <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
            <button data-testid={`routing-save-${row.role}`} style={primaryBtn} disabled={busy} onClick={save}>
              {busy ? 'Saving…' : 'Save'}
            </button>
            {row.configured && (
              <button data-testid={`routing-clear-${row.role}`} style={smallBtn} disabled={busy} onClick={clear}>
                Clear
              </button>
            )}
            <button style={smallBtn} disabled={busy} onClick={() => { setEditing(false); setError(null); }}>
              Cancel
            </button>
          </div>
        </div>
      )}
      {error && <div style={{ fontSize: 11, color: colors.danger }}>{error}</div>}
    </div>
  );
}

export function RoutingSection() {
  const { colors } = useTheme();
  const [data, setData] = useState<RoutingRolesResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    setError(null);
    // Deferred so a synchronous throw (older daemon build without the route,
    // or a harness whose api mock lacks it) lands in the catch, not the render.
    Promise.resolve()
      .then(() => api.getRoutingRoles())
      .then(setData)
      .catch(e => setError(`Couldn't load routing: ${e instanceof Error ? e.message : String(e)}`));
  }, []);

  useEffect(() => { load(); }, [load]);

  const onSaved = (updated: RoutingRoleRow) => {
    setData(d => d ? { ...d, roles: d.roles.map(r => (r.role === updated.role ? updated : r)) } : d);
  };

  return (
    <Section
      title="Routing"
      sub="Hand-set roles win. Otherwise the router derives a best-fit map from the models you actually have — local or cloud — cheapest that clears the role's bar. The main chat loop always stays on your session model."
    >
      {error && (
        <div style={{ fontSize: 12, color: colors.danger, marginBottom: 10 }}>
          {error} <button style={{ background: 'none', border: 'none', color: colors.cyan, cursor: 'pointer', fontSize: 12 }} onClick={load}>Retry</button>
        </div>
      )}
      {!data && !error && (
        <div style={{ fontSize: 13, color: colors.textDim }}>Loading routing…</div>
      )}
      {data && (
        <>
          {data.kb.stale && (
            <div
              data-testid="routing-kb-stale"
              style={{ fontSize: 11, color: colors.warning, marginBottom: 10, lineHeight: 1.5 }}
            >
              The model knowledge base this map is derived from is a snapshot from {data.kb.snapshot_date} and is over 90 days old — derived picks are estimates until it is refreshed.
            </div>
          )}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            {data.roles.map(row => (
              <RoleRowView key={row.role} row={row} onSaved={onSaved} colors={colors} />
            ))}
          </div>
          <div style={{ fontSize: 11, color: colors.textDim, marginTop: 12, lineHeight: 1.5, fontFamily: font.mono }}>
            {data.discovered.providers.length > 0
              ? `Discovered providers: ${data.discovered.providers.join(', ')}`
              : 'No providers discovered — add an API key or run Ollama.'}
            {data.discovered.local_models.length > 0 && ` · local models: ${data.discovered.local_models.join(', ')}`}
          </div>
        </>
      )}
    </Section>
  );
}

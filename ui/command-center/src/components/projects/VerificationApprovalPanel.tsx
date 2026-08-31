/**
 * VerificationApprovalPanel — this project's Settings for the verification
 * approval ladder (Tier 1 auto / Tier 2 user / earned privilege).
 *
 * Lives on the Details lens (ProjectDetails.tsx), the project's "everything
 * you read, edit, add or remove" surface — there is no separate per-project
 * Settings tab in this app (global Settings has no project scope at all), so
 * this bag of project-scoped config belongs where every other project-scoped
 * record lives, alongside StackPanel (also settings-shaped: reference config
 * with add/remove, no separate destination). One concept, one place: the
 * allowlist and thresholds are editable ONLY here — never via the Decision
 * Inbox card, never via a raw metadataJson PATCH elsewhere.
 *
 * Round-trips through GET /api/projects/{id} (read) and the merge-write
 * PUT /api/projects/{id}/verification-approval (write) via
 * verificationApproval.ts. Failures surface inline (#568 no-silent-catch).
 */

import { useCallback, useEffect, useState } from 'react';
import { FiPlus, FiTrash2, FiX } from 'react-icons/fi';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { formatAge } from '../dashboard/decisions/format';
import { Panel } from './Panel';
import {
  derivePrivilegeLevel,
  fetchVerificationApproval,
  privilegeLevelBlurb,
  saveVerificationApproval,
  type VerificationApproval,
} from './verificationApproval';
import type { Project } from './types';
import { GLOSSARY } from '../../lib/vocabulary';

export function VerificationApprovalPanel({ project }: { project: Project }) {
  const { colors, theme } = useTheme();
  const rowVeil = theme === 'silver' ? 'rgba(30,37,48,0.03)' : 'rgba(255,255,255,0.02)';

  const [data, setData] = useState<VerificationApproval | null>(null);
  const [status, setStatus] = useState<'loading' | 'error' | 'ready'>('loading');
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // Editable drafts, separate from `data` so unsaved edits don't vanish on a
  // background refetch and a save only ships fields that actually changed.
  const [readOnlyDraft, setReadOnlyDraft] = useState('');
  const [fullDraft, setFullDraft] = useState('');
  const [newEntry, setNewEntry] = useState('');
  const [showAllAudit, setShowAllAudit] = useState(false);

  const load = useCallback(async () => {
    try {
      const va = await fetchVerificationApproval(project.id);
      setData(va);
      setReadOnlyDraft(String(va.readOnlyThreshold));
      setFullDraft(String(va.fullThreshold));
      setStatus('ready');
      setError(null);
    } catch (e) {
      // Keep the reason: "couldn't load" with no cause is the silent catch
      // this file's header promises not to do.
      setError((e as Error).message || 'request failed');
      setStatus('error');
    }
  }, [project.id]);

  useEffect(() => { load(); }, [load]);

  const addEntry = async () => {
    const token = newEntry.trim();
    if (!data || !token) return;
    if (data.allowlist.includes(token)) { setNewEntry(''); return; }
    setSaving(true);
    setError(null);
    try {
      const next = [...data.allowlist, token];
      const updated = await saveVerificationApproval(project.id, { allowlist: next });
      setData(updated);
      setNewEntry('');
    } catch (e) {
      setError(`Couldn't save: ${(e as Error).message || 'request failed'}`);
    } finally {
      setSaving(false);
    }
  };

  const removeEntry = async (token: string) => {
    if (!data) return;
    setSaving(true);
    setError(null);
    try {
      const next = data.allowlist.filter(t => t !== token);
      const updated = await saveVerificationApproval(project.id, { allowlist: next });
      setData(updated);
    } catch (e) {
      setError(`Couldn't remove: ${(e as Error).message || 'request failed'}`);
    } finally {
      setSaving(false);
    }
  };

  const saveThresholds = async () => {
    if (!data) return;
    const readOnlyThreshold = Number(readOnlyDraft);
    const fullThreshold = Number(fullDraft);
    const whole = (n: number) => Number.isInteger(n) && n >= 0;
    if (!whole(readOnlyThreshold) || !whole(fullThreshold)) {
      // A threshold counts clean runs, so it is a whole number of them. Caught
      // here rather than as a 400 from serde's u32, which would say less.
      setError('Thresholds must be whole numbers, zero or more.');
      return;
    }
    if (fullThreshold > 0 && readOnlyThreshold > 0 && fullThreshold < readOnlyThreshold) {
      setError('The full threshold cannot be lower than the read-only one.');
      return;
    }
    const changes: { readOnlyThreshold?: number; fullThreshold?: number } = {};
    if (readOnlyThreshold !== data.readOnlyThreshold) changes.readOnlyThreshold = readOnlyThreshold;
    if (fullThreshold !== data.fullThreshold) changes.fullThreshold = fullThreshold;
    if (Object.keys(changes).length === 0) return;
    setSaving(true);
    setError(null);
    try {
      const updated = await saveVerificationApproval(project.id, changes);
      setData(updated);
      setReadOnlyDraft(String(updated.readOnlyThreshold));
      setFullDraft(String(updated.fullThreshold));
    } catch (e) {
      setError(`Couldn't save thresholds: ${(e as Error).message || 'request failed'}`);
    } finally {
      setSaving(false);
    }
  };

  const reset = async () => {
    setSaving(true);
    setError(null);
    try {
      const updated = await saveVerificationApproval(project.id, { reset: true });
      setData(updated);
      setReadOnlyDraft(String(updated.readOnlyThreshold));
      setFullDraft(String(updated.fullThreshold));
      setNewEntry('');
    } catch (e) {
      setError(`Couldn't reset: ${(e as Error).message || 'request failed'}`);
    } finally {
      setSaving(false);
    }
  };

  const inputStyle: React.CSSProperties = {
    fontSize: 12, padding: '6px 9px', borderRadius: 7,
    background: colors.inputBg, border: `1px solid ${colors.border}`,
    color: colors.text, fontFamily: font.body, outline: 'none',
  };

  if (status === 'loading') {
    return (
      <Panel title="Verification approval">
        <div style={{ fontSize: 11, color: colors.textDim }}>Loading…</div>
      </Panel>
    );
  }

  if (status === 'error' || !data) {
    return (
      <Panel title="Verification approval">
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <span style={{ fontSize: 11, color: colors.danger }}>
            Couldn't load verification approval settings{error ? `: ${error}` : '.'}
          </span>
          <button
            onClick={load}
            style={{
              fontSize: 11, color: colors.cyan, background: 'none', border: 'none',
              cursor: 'pointer', fontFamily: font.body, padding: 0, fontWeight: 600,
            }}
          >
            Retry
          </button>
        </div>
      </Panel>
    );
  }

  const level = derivePrivilegeLevel(data.cleanRuns, data.readOnlyThreshold, data.fullThreshold);
  const levelColor = level === 'full' ? colors.success : level === 'read_only' ? colors.warning : colors.textDim;
  const auditNewestFirst = [...data.audit].reverse();
  const visibleAudit = showAllAudit ? auditNewestFirst : auditNewestFirst.slice(0, 8);

  return (
    <Panel
      title="Verification approval"
      action={
        <button
          onClick={reset}
          disabled={saving}
          style={{
            background: 'none', border: 'none', padding: 0, cursor: saving ? 'default' : 'pointer',
            color: colors.textDim, fontFamily: font.body, fontSize: 11, fontWeight: 600,
            opacity: saving ? 0.5 : 1,
          }}
        >
          Reset
        </button>
      }
    >
      {error && (
        <div style={{ fontSize: 11, color: colors.danger, marginBottom: 8 }}>{error}</div>
      )}

      {/* Earned privilege — current level, the count, and what it permits. */}
      <div
        data-testid="privilege-level"
        data-level={level}
        style={{
          display: 'flex', alignItems: 'baseline', gap: 8, marginBottom: 4,
        }}
      >
        <span style={{
          fontSize: 11, fontWeight: 700, color: levelColor, textTransform: 'uppercase',
          letterSpacing: '0.04em',
        }}>
          {level === 'full' ? 'Full' : level === 'read_only' ? 'Read-only' : 'None'}
        </span>
        <span
          data-testid="clean-runs"
          title={GLOSSARY.cleanRuns}
          style={{ fontSize: 11, color: colors.textDim, fontFamily: font.mono, cursor: 'help' }}
        >
          {data.cleanRuns} clean run{data.cleanRuns === 1 ? '' : 's'}
        </span>
      </div>
      <div style={{ fontSize: 11, color: colors.textMuted, lineHeight: 1.5, marginBottom: 12 }}>
        {privilegeLevelBlurb(level)}
      </div>

      {/* Thresholds — editable numbers */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12, flexWrap: 'wrap' }}>
        <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11, color: colors.textDim }}>
          Read-only at
          <input
            type="number"
            min={0}
            value={readOnlyDraft}
            onChange={e => setReadOnlyDraft(e.target.value)}
            style={{ ...inputStyle, width: 56 }}
          />
        </label>
        <label style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11, color: colors.textDim }}>
          Full at
          <input
            type="number"
            min={0}
            value={fullDraft}
            onChange={e => setFullDraft(e.target.value)}
            style={{ ...inputStyle, width: 56 }}
          />
        </label>
        <button
          onClick={saveThresholds}
          disabled={saving}
          style={{
            fontSize: 11, fontWeight: 600, padding: '5px 12px', borderRadius: 7,
            cursor: saving ? 'default' : 'pointer', opacity: saving ? 0.5 : 1,
            background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`,
            color: colors.cyan, fontFamily: font.body,
          }}
        >
          Save thresholds
        </button>
      </div>

      {/* Allowlist — add/remove */}
      <div style={{ fontSize: 10, fontWeight: 600, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.06em', marginBottom: 6 }}>
        Allowlist
      </div>
      <div style={{ display: 'flex', gap: 6, marginBottom: 8 }}>
        <input
          value={newEntry}
          onChange={e => setNewEntry(e.target.value)}
          onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); addEntry(); } }}
          placeholder="Command token to allow — e.g. cargo"
          style={{ ...inputStyle, flex: 1, minWidth: 0 }}
        />
        <button
          onClick={addEntry}
          disabled={saving || !newEntry.trim()}
          style={{
            display: 'inline-flex', alignItems: 'center', gap: 4,
            fontSize: 11, fontWeight: 600, padding: '6px 10px', borderRadius: 7,
            cursor: (saving || !newEntry.trim()) ? 'default' : 'pointer',
            opacity: (saving || !newEntry.trim()) ? 0.5 : 1,
            background: colors.cyanSoft, border: `1px solid ${colors.borderHi}`,
            color: colors.cyan, fontFamily: font.body,
          }}
        >
          <FiPlus size={11} /> Add
        </button>
      </div>

      {data.allowlist.length === 0 ? (
        <div style={{ fontSize: 11, color: colors.textDim, marginBottom: 12 }}>
          Nothing allowlisted yet — every command outside earned privilege asks first.
        </div>
      ) : (
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6, marginBottom: 12 }}>
          {data.allowlist.map(token => (
            <span
              key={token}
              style={{
                display: 'inline-flex', alignItems: 'center', gap: 6,
                borderRadius: radius.pill, padding: '3px 6px 3px 10px',
                background: rowVeil, border: `1px solid ${colors.border}`,
                fontFamily: font.mono, fontSize: 11, color: colors.text,
              }}
            >
              {token}
              <button
                onClick={() => removeEntry(token)}
                title={`Remove ${token}`}
                disabled={saving}
                style={{
                  background: 'none', border: 'none', color: colors.textDim,
                  cursor: saving ? 'default' : 'pointer', display: 'flex', padding: 2,
                }}
              >
                <FiTrash2 size={11} />
              </button>
            </span>
          ))}
        </div>
      )}

      {/* Recent audit */}
      <div style={{ fontSize: 10, fontWeight: 600, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '0.06em', marginBottom: 6 }}>
        Recent activity
      </div>
      {visibleAudit.length === 0 ? (
        <div style={{ fontSize: 11, color: colors.textDim }}>No checks recorded yet.</div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {visibleAudit.map((row, i) => (
            <div
              key={`${row.at}-${i}`}
              style={{
                padding: '7px 9px', borderRadius: 7, background: rowVeil,
                border: `1px solid ${colors.border}`,
              }}
            >
              <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
                <span style={{
                  fontFamily: font.mono, fontSize: 11, color: colors.text, flex: 1, minWidth: 0,
                  overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                }}>
                  {row.command}
                </span>
                {row.at && (
                  <span style={{ fontSize: 10, color: colors.textDim, flexShrink: 0 }}>
                    {formatAge(row.at)}
                  </span>
                )}
              </div>
              <div style={{ fontSize: 10, color: colors.textMuted, marginTop: 2 }}>
                {row.decision.replace(/_/g, ' ')}
                {row.reason ? ` — ${row.reason}` : ''}
              </div>
            </div>
          ))}
        </div>
      )}
      {auditNewestFirst.length > 8 && (
        <button
          onClick={() => setShowAllAudit(s => !s)}
          style={{
            marginTop: 8, background: 'none', border: 'none', padding: 0,
            color: colors.cyan, fontFamily: font.body, fontSize: 11, fontWeight: 600,
            cursor: 'pointer', display: 'inline-flex', alignItems: 'center', gap: 4,
          }}
        >
          {showAllAudit ? <><FiX size={11} /> Show fewer</> : `Show all ${auditNewestFirst.length}`}
        </button>
      )}
    </Panel>
  );
}

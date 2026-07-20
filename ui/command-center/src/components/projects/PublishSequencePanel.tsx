import { useState, type CSSProperties } from 'react';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Panel } from './Panel';
import { readPublishSequence, savePublishSequence, type PublishStep } from './publishSequence';
import type { Project } from './types';

// ── Publish sequence panel (#457) ───────────────────────────────────────────
//
// The project's ordered post-push steps — what still has to run after a git
// push before a change is actually LIVE (seed prod DB, redeploy, …). Stored
// in `metadata_json.publish_sequence` and edited in place through the
// existing PATCH /api/projects/:id (no new endpoints); saves merge over a
// fresh GET so sibling metadata keys (build_command, …) are never clobbered
// (see publishSequence.ts). The orchestrator reads the same key: dispatched
// workers are told push ≠ live, and the review decision flags an un-run
// sequence.
//
// Draft state is initialized on entering edit mode only, so the 5s projects
// poll can't clobber typing (same pattern as the Overview summary editor).

/** One editable row of the draft — command text + timeout text (freeform
 *  while typing; validated/normalized on save). */
interface DraftStep {
  command: string;
  timeout: string;
}

function toDraft(steps: PublishStep[]): DraftStep[] {
  return steps.map(s => ({
    command: s.command,
    timeout: s.timeoutSecs !== undefined ? String(s.timeoutSecs) : '',
  }));
}

function fromDraft(rows: DraftStep[]): PublishStep[] {
  return rows
    .map(r => {
      const t = parseInt(r.timeout.trim(), 10);
      return {
        command: r.command.trim(),
        ...(Number.isFinite(t) && t > 0 ? { timeoutSecs: t } : {}),
      };
    })
    .filter(s => s.command !== '');
}

export function PublishSequencePanel({ project, onProjectUpdated }: {
  project: Project;
  /** Parent refetch after an edit persists (ProjectsView.loadProjects). */
  onProjectUpdated?: () => void;
}) {
  const { colors } = useTheme();
  // Prefer the just-saved copy over a stale project prop (the parent poll can
  // lag ≤5s behind a save). The override expires as soon as the prop catches
  // up — `updatedAt` is bumped by every PATCH, and RFC3339 strings from the
  // daemon compare lexicographically — so a later edit from anywhere else is
  // never masked.
  const [savedOverride, setSavedOverride] = useState<{ steps: PublishStep[]; updatedAt: string } | null>(null);
  const steps = savedOverride && project.updatedAt < savedOverride.updatedAt
    ? savedOverride.steps
    : readPublishSequence(project.metadataJson);

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<DraftStep[]>([]);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const startEditing = () => {
    setDraft(steps.length > 0 ? toDraft(steps) : [{ command: '', timeout: '' }]);
    setSaveError(null);
    setEditing(true);
  };

  const updateRow = (i: number, patch: Partial<DraftStep>) => {
    setDraft(d => d.map((row, j) => (j === i ? { ...row, ...patch } : row)));
  };
  const removeRow = (i: number) => setDraft(d => d.filter((_, j) => j !== i));
  const moveRow = (i: number, dir: -1 | 1) => {
    setDraft(d => {
      const j = i + dir;
      if (j < 0 || j >= d.length) return d;
      const next = [...d];
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    });
  };

  const save = async () => {
    setSaving(true);
    setSaveError(null);
    try {
      const updated = await savePublishSequence(project.id, fromDraft(draft));
      setSavedOverride({
        steps: readPublishSequence(updated.metadataJson),
        updatedAt: updated.updatedAt,
      });
      setEditing(false);
      onProjectUpdated?.();
    } catch (e) {
      // Keep the draft on screen — a failed save must not eat the user's steps.
      setSaveError(e instanceof Error ? e.message : 'Save failed');
    } finally {
      setSaving(false);
    }
  };

  const btn = (primary: boolean): CSSProperties => ({
    padding: '4px 12px', borderRadius: 6, cursor: saving ? 'default' : 'pointer',
    background: primary ? colors.cyanSoft : 'rgba(255,255,255,0.03)',
    border: `1px solid ${primary ? colors.borderHi : colors.border}`,
    color: primary ? colors.cyan : colors.textMuted,
    fontFamily: font.body, fontSize: 11, fontWeight: 600,
    opacity: saving ? 0.6 : 1,
  });
  const tinyBtn: CSSProperties = {
    padding: '2px 6px', borderRadius: 5, cursor: 'pointer', lineHeight: 1,
    background: 'rgba(255,255,255,0.03)', border: `1px solid ${colors.border}`,
    color: colors.textMuted, fontFamily: font.body, fontSize: 11,
  };

  return (
    <Panel
      title="Publish sequence"
      action={!editing ? (
        <button onClick={startEditing} style={btn(false)}>
          {steps.length > 0 ? 'Edit' : 'Add'}
        </button>
      ) : undefined}
    >
      {editing ? (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <div style={{ fontSize: 11, color: colors.textDim, lineHeight: 1.5 }}>
            Ordered steps run after commit + push before the change is live.
            Commands may source secrets from the project's .env.local — never
            paste secret values here.
          </div>
          {draft.map((row, i) => (
            <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
              <span style={{ fontSize: 11, color: colors.textDim, width: 14, flexShrink: 0, textAlign: 'right' }}>
                {i + 1}.
              </span>
              <input
                value={row.command}
                onChange={e => updateRow(i, { command: e.target.value })}
                placeholder="e.g. npx tsx scripts/reseed-threads.ts"
                disabled={saving}
                aria-label={`Step ${i + 1} command`}
                style={{
                  flex: 1, minWidth: 0, padding: '6px 8px', borderRadius: 6,
                  background: 'rgba(255,255,255,0.03)', border: `1px solid ${colors.border}`,
                  color: colors.text, fontFamily: font.mono, fontSize: 11, outline: 'none',
                }}
              />
              <input
                value={row.timeout}
                onChange={e => updateRow(i, { timeout: e.target.value })}
                placeholder="timeout s"
                disabled={saving}
                aria-label={`Step ${i + 1} timeout in seconds`}
                style={{
                  width: 62, flexShrink: 0, padding: '6px 8px', borderRadius: 6,
                  background: 'rgba(255,255,255,0.03)', border: `1px solid ${colors.border}`,
                  color: colors.text, fontFamily: font.mono, fontSize: 11, outline: 'none',
                }}
              />
              <button onClick={() => moveRow(i, -1)} disabled={saving || i === 0} aria-label={`Move step ${i + 1} up`} style={tinyBtn}>↑</button>
              <button onClick={() => moveRow(i, 1)} disabled={saving || i === draft.length - 1} aria-label={`Move step ${i + 1} down`} style={tinyBtn}>↓</button>
              <button onClick={() => removeRow(i)} disabled={saving} aria-label={`Remove step ${i + 1}`} style={{ ...tinyBtn, color: colors.warning }}>✕</button>
            </div>
          ))}
          <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
            <button
              onClick={() => setDraft(d => [...d, { command: '', timeout: '' }])}
              disabled={saving}
              style={btn(false)}
            >
              + Add step
            </button>
            <div style={{ flex: 1 }} />
            <button onClick={() => setEditing(false)} disabled={saving} style={btn(false)}>Cancel</button>
            <button onClick={save} disabled={saving} style={btn(true)}>
              {saving ? 'Saving…' : 'Save'}
            </button>
          </div>
          {saveError && (
            <div style={{ fontSize: 11, color: colors.warning }}>{saveError}</div>
          )}
        </div>
      ) : steps.length === 0 ? (
        <div style={{ fontSize: 11, color: colors.textDim, lineHeight: 1.5 }}>
          None — a git push is treated as live. If going live needs more
          (seed the prod DB, redeploy…), add the ordered steps here so agents
          stop reporting "pushed" as "live".
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          <div style={{ fontSize: 11, color: colors.textDim, lineHeight: 1.5 }}>
            After push, these run in order before the change is live:
          </div>
          {steps.map((s, i) => (
            <div key={i} style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
              <span style={{ fontSize: 11, color: colors.textDim, flexShrink: 0 }}>{i + 1}.</span>
              <span style={{
                fontFamily: font.mono, fontSize: 11, color: colors.text, minWidth: 0,
                overflowWrap: 'anywhere',
              }}>
                {s.command}
              </span>
              {s.timeoutSecs !== undefined && (
                <span style={{
                  fontSize: 9, color: colors.textDim, flexShrink: 0,
                  padding: '1px 6px', borderRadius: 4, background: 'rgba(255,255,255,0.06)',
                }}>
                  {s.timeoutSecs}s
                </span>
              )}
            </div>
          ))}
        </div>
      )}
    </Panel>
  );
}

import { useState, type CSSProperties } from 'react';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
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

  // Resolves `false` on failure: the reason is surfaced inline below the rows,
  // so the Save button must not tick over a sequence that never persisted.
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
      return true;
    } catch (e) {
      // Keep the draft on screen — a failed save must not eat the user's steps.
      setSaveError(e instanceof Error ? e.message : 'Save failed');
      return false;
    } finally {
      setSaving(false);
    }
  };

  // The look these controls had at rest, re-expressed as the primitive's custom
  // properties so `:hover` / `:active` / `:disabled` can finally reach them.
  const btn = (primary: boolean): CSSProperties => ({
    '--pa-btn-bg': primary ? colors.cyanSoft : colors.fillSubtle,
    '--pa-btn-fg': primary ? colors.cyan : colors.textMuted,
    '--pa-btn-border': primary ? colors.borderHi : colors.border,
    '--pa-btn-bg-hover': primary ? colors.cyanSoft : colors.surfaceHi,
    '--pa-btn-fg-hover': primary ? colors.cyan : colors.text,
    '--pa-btn-border-hover': primary ? colors.cyan : colors.borderHi,
    '--pa-btn-pad': '4px 12px',
    '--pa-btn-radius': `${radius.sm}px`,
    '--pa-btn-weight': 600,
    fontFamily: font.body,
    fontSize: textSize.micro,
  } as CSSProperties);
  const tinyBtn: CSSProperties = {
    '--pa-btn-bg': colors.fillSubtle,
    '--pa-btn-fg': colors.textMuted,
    '--pa-btn-border': colors.border,
    '--pa-btn-bg-hover': colors.surfaceHi,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-border-hover': colors.borderHi,
    '--pa-btn-pad': '2px 6px',
    // 5px, not a radius token: these three sit inside a 24px row and the
    // resting shape must not change under the migration.
    '--pa-btn-radius': `${radius.xs}px`,
    fontFamily: font.body,
    fontSize: textSize.micro,
    lineHeight: 1,
  } as CSSProperties;

  return (
    <Panel
      title="Publish sequence"
      action={!editing ? (
        <Button colors={colors} onClick={startEditing} style={btn(false)}>
          {steps.length > 0 ? 'Edit' : 'Add'}
        </Button>
      ) : undefined}
    >
      {editing ? (
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.md }}>
          <div style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>
            Ordered steps run after commit + push before the change is live.
            Commands may source secrets from the project's .env.local — never
            paste secret values here.
          </div>
          {draft.map((row, i) => (
            <div key={i} style={{ display: 'flex', alignItems: 'center', gap: space.sm }}>
              <span style={{ fontSize: textSize.micro, color: colors.textDim, width: 14, flexShrink: 0, textAlign: 'right' }}>
                {i + 1}.
              </span>
              <input
                value={row.command}
                onChange={e => updateRow(i, { command: e.target.value })}
                placeholder="e.g. npx tsx scripts/reseed-threads.ts"
                disabled={saving}
                aria-label={`Step ${i + 1} command`}
                style={{
                  flex: 1, minWidth: 0, padding: `${space.sm}px ${space.md}px`, borderRadius: radius.sm,
                  background: colors.fillSubtle, border: `1px solid ${colors.border}`,
                  color: colors.text, fontFamily: font.mono, fontSize: textSize.micro, outline: 'none',
                }}
              />
              <input
                value={row.timeout}
                onChange={e => updateRow(i, { timeout: e.target.value })}
                placeholder="timeout s"
                disabled={saving}
                aria-label={`Step ${i + 1} timeout in seconds`}
                style={{
                  width: 62, flexShrink: 0, padding: `${space.sm}px ${space.md}px`, borderRadius: radius.sm,
                  background: colors.fillSubtle, border: `1px solid ${colors.border}`,
                  color: colors.text, fontFamily: font.mono, fontSize: textSize.micro, outline: 'none',
                }}
              />
              <Button colors={colors} onClick={() => moveRow(i, -1)} disabled={saving || i === 0} aria-label={`Move step ${i + 1} up`} style={tinyBtn}>↑</Button>
              <Button colors={colors} onClick={() => moveRow(i, 1)} disabled={saving || i === draft.length - 1} aria-label={`Move step ${i + 1} down`} style={tinyBtn}>↓</Button>
              <Button colors={colors} onClick={() => removeRow(i)} disabled={saving} aria-label={`Remove step ${i + 1}`} style={{ ...tinyBtn, '--pa-btn-fg': colors.warning, '--pa-btn-fg-hover': colors.warning } as CSSProperties}>✕</Button>
            </div>
          ))}
          <div style={{ display: 'flex', gap: space.sm, alignItems: 'center' }}>
            <Button
              colors={colors}
              onClick={() => setDraft(d => [...d, { command: '', timeout: '' }])}
              disabled={saving}
              style={btn(false)}
            >
              + Add step
            </Button>
            <div style={{ flex: 1 }} />
            <Button colors={colors} onClick={() => setEditing(false)} disabled={saving} style={btn(false)}>Cancel</Button>
            <Button colors={colors} onClick={save} disabled={saving} style={btn(true)}>
              {saving ? 'Saving…' : 'Save'}
            </Button>
          </div>
          {saveError && (
            <div style={{ fontSize: textSize.micro, color: colors.warning }}>{saveError}</div>
          )}
        </div>
      ) : steps.length === 0 ? (
        <div style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>
          None — a git push is treated as live. If going live needs more
          (seed the prod DB, redeploy…), add the ordered steps here so agents
          stop reporting "pushed" as "live".
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: space.sm }}>
          <div style={{ fontSize: textSize.micro, color: colors.textDim, lineHeight: 1.5 }}>
            After push, these run in order before the change is live:
          </div>
          {steps.map((s, i) => (
            <div key={i} style={{ display: 'flex', alignItems: 'baseline', gap: space.md }}>
              <span style={{ fontSize: textSize.micro, color: colors.textDim, flexShrink: 0 }}>{i + 1}.</span>
              <span style={{
                fontFamily: font.mono, fontSize: textSize.micro, color: colors.text, minWidth: 0,
                overflowWrap: 'anywhere',
              }}>
                {s.command}
              </span>
              {s.timeoutSecs !== undefined && (
                <span style={{
                  fontSize: 10, color: colors.textDim, flexShrink: 0,
                  padding: `${space.xxs}px ${space.sm}px`, borderRadius: radius.xs, background: colors.fillHover,
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

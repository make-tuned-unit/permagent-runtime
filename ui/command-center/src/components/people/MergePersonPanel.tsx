/**
 * MergePersonPanel — the three-step merge flow for the CRM duplicate-cleanup
 * feature: pick the other record, preview exactly what moves, confirm.
 *
 * The `person` prop is the SURVIVOR by default (they keep their id — the
 * request path always uses whichever id is currently marked as survivor); the
 * picked person is absorbed and deleted. A "Swap" control flips the two, which
 * re-fetches the preview under the flipped ids rather than just relabeling —
 * the backend computes field/link winners from which side is the survivor.
 *
 * Self-fetching, presentational, inline-styled — same conventions as the rest
 * of the directory surface (PersonDetailModal, PeoplePanel).
 */

import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { apiFetch } from '../../lib/api';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import type { DirectoryPerson, DuplicateSuggestion, MergePreview, MergeReport, Person } from '../projects/types';

/** The minimal identity of "one side" of a merge — enough to label the UI and
 *  drive the request path, without needing the full Person record. */
interface Side {
  entity_uuid: string;
  display_name: string;
}

type Step = 'pick' | 'preview';

export function MergePersonPanel({
  person,
  onDone,
  onCancel,
}: {
  person: Person;
  onDone: (report: MergeReport) => void;
  onCancel: () => void;
}) {
  const { colors } = useTheme();

  const [step, setStep] = useState<Step>('pick');
  const [survivor, setSurvivor] = useState<Side>({ entity_uuid: person.entity_uuid, display_name: person.display_name });
  const [duplicate, setDuplicate] = useState<Side | null>(null);

  // ── Step 1: pick ──────────────────────────────────────────────────────
  const [query, setQuery] = useState('');
  const [directory, setDirectory] = useState<DirectoryPerson[]>([]);
  const [directoryStatus, setDirectoryStatus] = useState<'loading' | 'ready' | 'error'>('loading');
  const [suggestions, setSuggestions] = useState<DuplicateSuggestion[]>([]);
  const [suggestionsStatus, setSuggestionsStatus] = useState<'loading' | 'ready' | 'error'>('loading');

  useEffect(() => {
    let live = true;
    setDirectoryStatus('loading');
    apiFetch<DirectoryPerson[]>('/api/people/directory')
      .then(rows => {
        if (!Array.isArray(rows)) throw new Error('Invalid directory response');
        if (live) { setDirectory(rows); setDirectoryStatus('ready'); }
      })
      .catch(() => { if (live) setDirectoryStatus('error'); });
    return () => { live = false; };
  }, []);

  useEffect(() => {
    let live = true;
    setSuggestionsStatus('loading');
    // A larger-than-default limit so this person's own likely duplicates are
    // more likely to be inside the returned window (the endpoint has no
    // per-person filter — this panel filters the top N client-side).
    apiFetch<DuplicateSuggestion[]>('/api/people/duplicates?limit=50')
      .then(rows => {
        if (!Array.isArray(rows)) throw new Error('Invalid duplicates response');
        if (live) { setSuggestions(rows); setSuggestionsStatus('ready'); }
      })
      .catch(() => { if (live) setSuggestionsStatus('error'); });
    return () => { live = false; };
  }, []);

  const relevantSuggestions = suggestions
    .filter(row => row.survivor_uuid === person.entity_uuid || row.duplicate_uuid === person.entity_uuid)
    .map(row => {
      const other: Side = row.survivor_uuid === person.entity_uuid
        ? { entity_uuid: row.duplicate_uuid, display_name: row.duplicate_name }
        : { entity_uuid: row.survivor_uuid, display_name: row.survivor_name };
      return { other, score: row.score, reasons: row.reasons };
    });

  const filteredDirectory = directory.filter(p => {
    if (p.entity_uuid === person.entity_uuid) return false;
    const q = query.trim().toLowerCase();
    if (!q) return true;
    const hay = [p.display_name, p.company, p.role].filter(Boolean).join(' ').toLowerCase();
    return hay.includes(q);
  });

  const pick = (other: Side) => {
    setSurvivor({ entity_uuid: person.entity_uuid, display_name: person.display_name });
    setDuplicate(other);
    setStep('preview');
  };

  // ── Step 2: preview ───────────────────────────────────────────────────
  const [preview, setPreview] = useState<MergePreview | null>(null);
  const [previewStatus, setPreviewStatus] = useState<'idle' | 'loading' | 'ready' | 'error'>('idle');

  /** Resolves whether the preview landed: the Retry control awaits this, and a
   *  failure here is swallowed into `previewStatus` rather than thrown. */
  const loadPreview = useCallback(async (): Promise<boolean> => {
    if (!duplicate) return false;
    setPreviewStatus('loading');
    try {
      const data = await apiFetch<MergePreview>(
        `/api/people/${encodeURIComponent(survivor.entity_uuid)}/merge-preview?duplicate_id=${encodeURIComponent(duplicate.entity_uuid)}`,
      );
      setPreview(data);
      setPreviewStatus('ready');
      return true;
    } catch {
      setPreview(null);
      setPreviewStatus('error');
      return false;
    }
  }, [survivor.entity_uuid, duplicate]);

  useEffect(() => {
    if (step === 'preview') void loadPreview();
  }, [step, loadPreview]);

  const swap = () => {
    if (!duplicate) return;
    setSurvivor(duplicate);
    setDuplicate(survivor);
  };

  const backToPick = () => {
    setStep('pick');
    setDuplicate(null);
    setPreview(null);
    setPreviewStatus('idle');
    setConfirmError(null);
  };

  // ── Step 3: confirm ───────────────────────────────────────────────────
  const [confirming, setConfirming] = useState(false);
  const [confirmError, setConfirmError] = useState<string | null>(null);

  /** Resolves false when the merge did not happen — the error is swallowed into
   *  `confirmError` below, and a tick over the top of that would be a lie about
   *  a destructive action. */
  const confirmMerge = async (): Promise<boolean> => {
    if (!duplicate || previewStatus !== 'ready') return false;
    setConfirming(true);
    setConfirmError(null);
    try {
      const report = await apiFetch<MergeReport>(
        `/api/people/${encodeURIComponent(survivor.entity_uuid)}/merge`,
        { method: 'POST', body: JSON.stringify({ duplicate_id: duplicate.entity_uuid, confirm: true }) },
      );
      onDone(report);
      return true;
    } catch (e) {
      const err = e as Error & { status?: number };
      setConfirmError(`Couldn't merge: ${err.status ? `${err.status} ` : ''}${err.message || 'request failed'}`);
      return false;
    } finally {
      setConfirming(false);
    }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      {step === 'pick' && (
        <PickStep
          colors={colors}
          personName={person.display_name}
          query={query}
          onQuery={setQuery}
          suggestions={relevantSuggestions}
          suggestionsStatus={suggestionsStatus}
          candidates={filteredDirectory}
          directoryStatus={directoryStatus}
          onPick={pick}
          onCancel={onCancel}
        />
      )}
      {step === 'preview' && duplicate && (
        <PreviewStep
          colors={colors}
          survivor={survivor}
          duplicate={duplicate}
          preview={preview}
          status={previewStatus}
          confirming={confirming}
          confirmError={confirmError}
          onSwap={swap}
          onBack={backToPick}
          onRetry={loadPreview}
          onConfirm={confirmMerge}
          onCancel={onCancel}
        />
      )}
    </div>
  );
}

// ── Step 1: pick ─────────────────────────────────────────────────────────

function PickStep({
  colors, personName, query, onQuery, suggestions, suggestionsStatus, candidates, directoryStatus, onPick, onCancel,
}: {
  colors: ReturnType<typeof useTheme>['colors'];
  personName: string;
  query: string;
  onQuery: (v: string) => void;
  suggestions: { other: Side; score: number; reasons: string[] }[];
  suggestionsStatus: 'loading' | 'ready' | 'error';
  candidates: DirectoryPerson[];
  directoryStatus: 'loading' | 'ready' | 'error';
  onPick: (other: Side) => void;
  onCancel: () => void;
}) {
  return (
    <>
      <div style={{ fontSize: 12, color: colors.textMuted }}>
        Merge another record into <strong style={{ color: colors.text }}>{personName}</strong>. Pick who they're the same person as.
      </div>

      <SectionLabel colors={colors}>Likely duplicates</SectionLabel>
      {suggestionsStatus === 'loading' && <Small colors={colors}>Looking for likely duplicates…</Small>}
      {suggestionsStatus === 'error' && <Small colors={colors}>Couldn't load duplicate suggestions.</Small>}
      {suggestionsStatus === 'ready' && suggestions.length === 0 && (
        <Small colors={colors}>No likely duplicates found for {personName}.</Small>
      )}
      {suggestions.map(({ other, score, reasons }) => (
        // `display: contents` on the primitive's label wrapper so the name,
        // the score and the reasons stay the row's own baseline-aligned flex
        // children rather than being boxed together.
        <Button
          key={other.entity_uuid}
          colors={colors}
          onClick={() => onPick(other)}
                    style={rowBtn(colors)}
        >
          <span style={{ fontSize: 12, color: colors.text, fontWeight: 600 }}>{other.display_name}</span>
          <span style={{ fontSize: 11, color: colors.cyan, fontFamily: font.mono, flexShrink: 0 }}>{Math.round(score * 100)}%</span>
          {reasons.length > 0 && (
            <span style={{ fontSize: 11, color: colors.textDim, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {reasons.join(', ')}
            </span>
          )}
        </Button>
      ))}

      <SectionLabel colors={colors}>Or search the directory</SectionLabel>
      <input
        autoFocus
        value={query}
        onChange={e => onQuery(e.target.value)}
        placeholder="Search people…"
        aria-label="Search people to merge"
        style={inputStyle(colors)}
      />
      {directoryStatus === 'loading' && <Small colors={colors}>Loading directory…</Small>}
      {directoryStatus === 'error' && <Small colors={colors}>Couldn't load the directory.</Small>}
      {directoryStatus === 'ready' && candidates.length === 0 && (
        <Small colors={colors}>No matches.</Small>
      )}
      {directoryStatus === 'ready' && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 2, maxHeight: 200, overflowY: 'auto' }}>
          {candidates.map(p => (
            <Button
              key={p.entity_uuid}
              colors={colors}
              onClick={() => onPick({ entity_uuid: p.entity_uuid, display_name: p.display_name })}
                            style={rowBtn(colors)}
            >
              <span style={{ fontSize: 12, color: colors.text }}>{p.display_name}</span>
              <span style={{ fontSize: 11, color: colors.textDim, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {[p.role, p.company].filter(Boolean).join(' · ')}
              </span>
            </Button>
          ))}
        </div>
      )}

      <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
        <Button colors={colors} onClick={onCancel} style={ghostBtn(colors)}>Cancel</Button>
      </div>
    </>
  );
}

// ── Step 2: preview + confirm ────────────────────────────────────────────

function PreviewStep({
  colors, survivor, duplicate, preview, status, confirming, confirmError, onSwap, onBack, onRetry, onConfirm, onCancel,
}: {
  colors: ReturnType<typeof useTheme>['colors'];
  survivor: Side;
  duplicate: Side;
  preview: MergePreview | null;
  status: 'idle' | 'loading' | 'ready' | 'error';
  confirming: boolean;
  confirmError: string | null;
  onSwap: () => void;
  onBack: () => void;
  onRetry: () => void;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const disabled = status !== 'ready' || confirming;
  return (
    <>
      <div style={{ fontSize: 12, color: colors.text }}>
        Keep <strong>{survivor.display_name}</strong>, absorb <strong>{duplicate.display_name}</strong>.
      </div>
      <div style={{ display: 'flex', gap: 6 }}>
        <Button colors={colors} onClick={onBack} style={miniBtn(colors)}>Back</Button>
        <Button colors={colors} onClick={onSwap} style={miniBtn(colors)}>Swap: keep {duplicate.display_name} instead</Button>
      </div>

      {status === 'loading' && <Small colors={colors}>Loading merge preview…</Small>}
      {status === 'error' && (
        <Small colors={colors}>
          Couldn't load the merge preview.{' '}
          <Button colors={colors} variant="bare" onClick={onRetry} className="hover:underline" style={linkBtn(colors)}>Retry</Button>
        </Small>
      )}

      {status === 'ready' && preview && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div style={{ fontSize: 12, color: colors.text }}>
            {preview.meetings} meeting{preview.meetings === 1 ? '' : 's'} move
            {preview.open_follow_ups > 0 ? `, ${preview.open_follow_ups} with an open follow-up` : ''}.
          </div>

          <section>
            <SectionLabel colors={colors}>Project links</SectionLabel>
            {preview.project_links.length === 0 && <Small colors={colors}>No project links to move.</Small>}
            {preview.project_links.map(link => (
              <div key={link.project_id} style={{ fontSize: 11, color: colors.textMuted, padding: '3px 0' }}>
                <span style={{ color: colors.text }}>{link.project_name}</span>
                {link.role ? ` · ${link.role}` : ''}
                {' — '}
                {link.survivor_already_linked
                  ? <span style={{ color: colors.textDim }}>already there, dropped</span>
                  : <span style={{ color: colors.cyan }}>moves to {survivor.display_name}</span>}
              </div>
            ))}
          </section>

          <section>
            <SectionLabel colors={colors}>Fields copied onto {survivor.display_name}</SectionLabel>
            {preview.fields.length === 0 && <Small colors={colors}>No fields to copy.</Small>}
            {preview.fields.map(f => (
              <div key={f.field_name} style={{ fontSize: 11, color: colors.textMuted, padding: '3px 0' }}>
                <span style={{ color: colors.text }}>{f.field_name}</span> → {f.value}
                <span style={{ color: colors.textDim }}> ({f.source})</span>
              </div>
            ))}
            {preview.fields_kept_from_survivor.length > 0 && (
              <div style={{ fontSize: 11, color: colors.textDim, marginTop: 4 }}>
                Kept from {survivor.display_name}: {preview.fields_kept_from_survivor.join(', ')}
              </div>
            )}
          </section>

          {preview.aliases.length > 0 && (
            <section>
              <SectionLabel colors={colors}>Aliases recorded</SectionLabel>
              <Small colors={colors}>{preview.aliases.join(', ')}</Small>
            </section>
          )}

          <div style={{ fontSize: 11, color: colors.textDim }}>
            {preview.graph_edges} graph edge{preview.graph_edges === 1 ? '' : 's'} move.
          </div>

          {preview.retained.length > 0 && (
            <div style={{
              fontSize: 11, color: colors.textMuted, borderRadius: radius.md,
              border: `1px solid ${colors.border}`, padding: '8px 10px',
            }}>
              <div style={{ fontFamily: font.mono, fontSize: 10, color: colors.textDim, textTransform: 'uppercase', letterSpacing: '.04em', marginBottom: 4 }}>
                What stays put
              </div>
              {preview.retained.map((line, i) => <div key={i}>{line}</div>)}
            </div>
          )}
        </div>
      )}

      {confirmError && (
        <div style={{ fontSize: 12, color: colors.danger }}>{confirmError}</div>
      )}

      <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
        <Button colors={colors} onClick={onCancel} disabled={confirming} style={ghostBtn(colors)}>Cancel</Button>
        <Button colors={colors} onClick={onConfirm} disabled={disabled} style={dangerBtn(colors)}>
          {confirming ? 'Merging…' : `Merge and delete ${duplicate.display_name}`}
        </Button>
      </div>
    </>
  );
}

// ── shared bits ────────────────────────────────────────────────────────────

function SectionLabel({ colors, children }: { colors: ReturnType<typeof useTheme>['colors']; children: React.ReactNode }) {
  return <div style={{ fontSize: 11, color: colors.textDim, fontFamily: font.mono, textTransform: 'uppercase', letterSpacing: '.04em' }}>{children}</div>;
}
function Small({ colors, children }: { colors: ReturnType<typeof useTheme>['colors']; children: React.ReactNode }) {
  return <div style={{ fontSize: 11, color: colors.textDim, marginTop: 4 }}>{children}</div>;
}

function inputStyle(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    fontSize: 12, padding: '6px 9px', borderRadius: radius.md,
    background: 'rgba(255,255,255,0.03)', border: `1px solid ${colors.border}`,
    color: colors.text, fontFamily: font.body, outline: 'none', width: '100%', boxSizing: 'border-box',
  };
}

// The panel's five button faces. Each keeps the resting look it had as an
// inline style and hands it to `.pa-btn` as custom properties instead, because
// an inline `background`/`color` beats the `:hover` rule and would silently
// kill the state the primitive exists to add.

function rowBtn(colors: ReturnType<typeof useTheme>['colors']): CSSProperties {
  return {
    '--pa-btn-bg': 'rgba(255,255,255,0.02)',
    '--pa-btn-fg': colors.text,
    '--pa-btn-border': colors.border,
    '--pa-btn-bg-hover': colors.surfaceHi,
    '--pa-btn-border-hover': colors.borderHi,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-bg-active': colors.surface,
    '--pa-btn-pad': '6px 9px',
    '--pa-btn-radius': `${radius.md}px`,
    '--pa-btn-weight': 400,
    // The row lays its own name/score/reasons out on a shared baseline and
    // reads from the left edge; `.pa-btn` centres by default.
    alignItems: 'baseline',
    justifyContent: 'flex-start',
    gap: 8,
    textAlign: 'left',
    width: '100%',
    fontFamily: font.body,
  } as CSSProperties;
}

function miniBtn(colors: ReturnType<typeof useTheme>['colors']): CSSProperties {
  return {
    '--pa-btn-bg': 'transparent',
    '--pa-btn-fg': colors.textMuted,
    '--pa-btn-border': colors.border,
    '--pa-btn-bg-hover': colors.surfaceHi,
    '--pa-btn-border-hover': colors.borderHi,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-bg-active': colors.surface,
    '--pa-btn-pad': '4px 7px',
    '--pa-btn-radius': `${radius.md}px`,
    '--pa-btn-weight': 400,
    gap: 3,
    fontFamily: font.body,
    fontSize: 11,
  } as CSSProperties;
}

function ghostBtn(colors: ReturnType<typeof useTheme>['colors']): CSSProperties {
  return {
    '--pa-btn-bg': 'transparent',
    '--pa-btn-fg': colors.textMuted,
    '--pa-btn-border': colors.border,
    '--pa-btn-bg-hover': colors.surfaceHi,
    '--pa-btn-border-hover': colors.borderHi,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-bg-active': colors.surface,
    '--pa-btn-pad': '6px 14px',
    '--pa-btn-radius': `${radius.md}px`,
    '--pa-btn-weight': 400,
    fontFamily: font.body,
    fontSize: 12,
  } as CSSProperties;
}

function dangerBtn(colors: ReturnType<typeof useTheme>['colors']): CSSProperties {
  return {
    '--pa-btn-bg': colors.danger + '14',
    '--pa-btn-fg': colors.danger,
    '--pa-btn-border': colors.danger,
    '--pa-btn-bg-hover': colors.danger + '26',
    '--pa-btn-border-hover': colors.danger,
    '--pa-btn-fg-hover': colors.danger,
    '--pa-btn-bg-active': colors.danger + '38',
    '--pa-btn-pad': '6px 14px',
    '--pa-btn-radius': `${radius.md}px`,
    '--pa-btn-weight': 500,
    fontFamily: font.body,
    fontSize: 12,
  } as CSSProperties;
}

function linkBtn(colors: ReturnType<typeof useTheme>['colors']): CSSProperties {
  return {
    '--pa-btn-fg': colors.cyan,
    '--pa-btn-fg-hover': colors.cyan,
    '--pa-btn-bg-hover': 'transparent',
    '--pa-btn-bg-active': 'transparent',
    '--pa-btn-pad': '0',
    '--pa-btn-weight': 400,
    fontFamily: font.body,
    fontSize: 11,
  } as CSSProperties;
}

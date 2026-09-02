/**
 * CardDetailModal — the detail view for a Kanban card.
 *
 * Reported symptom: "clicking a card in Kanban should open the card so I can
 * see the details. Right now nothing happens when I click it." Nothing happened
 * because nothing existed: goal cards opened GoalDetailModal, and every other
 * card — the standard to-dos and Grow posts that make up most boards — had no
 * detail view at all. This is it.
 *
 * It is deliberately the sibling of GoalDetailModal rather than a new kind of
 * thing: same DetailModal shell (scrim, header, scrollable body, pinned
 * footer), same self-contained {projectId, cardId} fetch, same
 * escape/scrim-to-close. Goals keep their own modal because their surface is a
 * lifecycle (dependencies, evidence, cancellation); this one is the card as the
 * user wrote it.
 *
 * Editing writes to the SAME routes every other writer uses:
 *   - title + description → PATCH /api/projects/{pid}/cards/{cid}
 *   - due date            → PUT   /api/projects/{pid}/cards/{cid}/due-date
 * The due-date route is `cards::set_card_due_date`, which is also what the
 * agent's `card_create` / `card_update` due_date argument calls — so a date the
 * user sets here and one the orchestrator sets are the same write, validated
 * the same way, and the Home tab's to-do list cannot disagree with either.
 */

import { useCallback, useEffect, useState, type CSSProperties } from 'react';
import { apiFetch } from '../../lib/api';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';
import { DetailModal } from '../common/DetailModal';
import type { BoardColumn, Card } from './types';

function fmtTime(iso: string): string {
  const t = Date.parse(iso);
  return Number.isFinite(t) ? new Date(t).toLocaleString() : iso;
}

/**
 * Metadata keys this modal renders through a dedicated control or that are
 * machinery rather than content. Everything else in `metadataJson` is shown
 * verbatim in the Details block — a card carrying a field nobody surfaces is
 * how information goes missing.
 */
const HANDLED_METADATA_KEYS = new Set(['dueDate', 'dueDismissedAt']);

export function CardDetailModal({
  projectId,
  projectName,
  cardId,
  onClose,
  onSaved,
}: {
  projectId: string;
  /** Shown as the card's project; the board already knows it, so we don't refetch. */
  projectName: string;
  cardId: string;
  onClose: () => void;
  /** Called after a persisted edit so the board behind the modal refreshes. */
  onSaved?: () => void;
}) {
  const { colors } = useTheme();
  const [card, setCard] = useState<Card | null>(null);
  const [columns, setColumns] = useState<BoardColumn[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(false);

  const [editing, setEditing] = useState(false);
  const [draftTitle, setDraftTitle] = useState('');
  const [draftDescription, setDraftDescription] = useState('');
  const [draftDueDate, setDraftDueDate] = useState('');
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    setLoading(true);
    setLoadError(false);
    Promise.all([
      apiFetch<Card>(`/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(cardId)}`),
      apiFetch<BoardColumn[]>(`/api/projects/${encodeURIComponent(projectId)}/columns`)
        .catch(() => [] as BoardColumn[]),
    ])
      .then(([c, cols]) => {
        if (!live) return;
        setCard(c);
        setColumns(cols);
      })
      .catch(() => { if (live) setLoadError(true); })
      .finally(() => { if (live) setLoading(false); });
    return () => { live = false; };
  }, [projectId, cardId]);

  const dueDate = typeof card?.metadataJson?.dueDate === 'string' ? card.metadataJson.dueDate : '';
  const columnName = columns.find(c => c.id === card?.columnId)?.name ?? card?.columnId ?? '—';

  const startEdit = useCallback(() => {
    setDraftTitle(card?.title ?? '');
    setDraftDescription(card?.description ?? '');
    setDraftDueDate(dueDate);
    setSaveError(null);
    setEditing(true);
  }, [card, dueDate]);

  // Resolves true/false rather than void: the failure is caught here and shown
  // inline, so a Save button that could not tell the two apart would confirm a
  // save that never happened.
  const save = useCallback(async (): Promise<boolean> => {
    if (!card) return false;
    const title = draftTitle.trim();
    if (!title) {
      setSaveError('A card needs a title.');
      return false;
    }
    setSaving(true);
    setSaveError(null);
    try {
      // Two writes because they are two different facts about the card: the
      // due date goes through its own route so the dismissal-clearing and
      // format-validation rules live in one place rather than being restated
      // by every caller that can PATCH metadata.
      if (title !== card.title || draftDescription !== card.description) {
        await apiFetch<Card>(
          `/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(cardId)}`,
          {
            method: 'PATCH',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ title, description: draftDescription }),
          },
        );
      }
      if (draftDueDate !== dueDate) {
        await apiFetch(
          `/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(cardId)}/due-date`,
          {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ dueDate: draftDueDate || null }),
          },
        );
      }
      // Re-read rather than patch local state: the due-date route also clears a
      // dismissal, so what persisted is a superset of what we sent.
      const fresh = await apiFetch<Card>(
        `/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(cardId)}`,
      );
      setCard(fresh);
      setEditing(false);
      onSaved?.();
      return true;
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : 'Saving the card failed.');
      return false;
    } finally {
      setSaving(false);
    }
  }, [card, cardId, draftDescription, draftDueDate, draftTitle, dueDate, onSaved, projectId]);

  const extraMetadata = Object.entries(card?.metadataJson ?? {})
    .filter(([k]) => !HANDLED_METADATA_KEYS.has(k));

  const badge = card
    ? { label: card.cardType, color: colors.cyan, bg: colors.cyanSoft }
    : null;

  const footer = card && !loading ? (
    editing ? (
      <>
        <Button colors={colors} onClick={() => setEditing(false)} disabled={saving} style={ghostVars(colors)}>
          Discard
        </Button>
        <Button
          colors={colors}
          onClick={save}
          disabled={saving}
          style={{
            ...ghostVars(colors),
            '--pa-btn-fg': colors.cyan,
            '--pa-btn-fg-hover': colors.cyan,
            '--pa-btn-border': colors.cyan,
            '--pa-btn-border-hover': colors.cyan,
            '--pa-btn-bg-hover': colors.cyanSoft,
            '--pa-btn-bg-active': colors.cyanGlow,
          } as CSSProperties}
        >
          {saving ? 'Saving…' : 'Save changes'}
        </Button>
      </>
    ) : (
      <Button colors={colors} onClick={startEdit} style={ghostVars(colors)}>Edit card</Button>
    )
  ) : null;

  return (
    <DetailModal title={card?.title ?? 'Card'} badge={badge} onClose={onClose} footer={footer}>
      {loading && <div style={{ fontSize: textSize.caption, color: colors.textMuted }}>Loading card…</div>}

      {loadError && !loading && (
        <div style={{
          fontSize: textSize.caption, color: colors.danger,
          borderRadius: radius.md, border: `1px solid ${colors.danger}`,
          background: colors.danger + '14', padding: '8px 12px',
        }}>
          Couldn&apos;t load this card. Check the daemon connection and try again.
        </div>
      )}

      {card && !loading && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
          {editing ? (
            <>
              <Field label="Title">
                <input
                  aria-label="Card title"
                  value={draftTitle}
                  onChange={e => setDraftTitle(e.target.value)}
                  style={inputStyle(colors)}
                />
              </Field>
              <Field label="Description">
                <textarea
                  aria-label="Card description"
                  value={draftDescription}
                  onChange={e => setDraftDescription(e.target.value)}
                  rows={5}
                  style={{ ...inputStyle(colors), resize: 'vertical' }}
                />
              </Field>
              <Field label="Due date">
                <input
                  type="date"
                  aria-label="Card due date"
                  value={draftDueDate}
                  onChange={e => setDraftDueDate(e.target.value)}
                  style={{ ...inputStyle(colors), width: 200 }}
                />
                <div style={{ fontSize: textSize.micro, color: colors.textDim, marginTop: 4 }}>
                  A to-do only reaches the Home tab&apos;s list once it has a due date. Clear the
                  field to take it off that list; the card stays on the board either way.
                </div>
              </Field>
            </>
          ) : (
            <>
              <Field label="Description">
                <div style={{ fontSize: textSize.caption, color: card.description ? colors.text : colors.textDim, whiteSpace: 'pre-wrap', userSelect: 'text' }}>
                  {card.description || 'No description.'}
                </div>
              </Field>
              <Field label="Details">
                <MetaGrid
                  colors={colors}
                  rows={[
                    ['Project', projectName],
                    ['Column', columnName],
                    ['Type', card.cardType],
                    ['Assignee', card.assignedTo ?? 'Unassigned'],
                    ['Due', dueDate || 'No due date'],
                    ['Created', fmtTime(card.createdAt)],
                    ['Updated', fmtTime(card.updatedAt)],
                    ...extraMetadata.map(([k, v]): [string, string] => [
                      k,
                      typeof v === 'string' ? v : JSON.stringify(v),
                    ]),
                  ]}
                />
              </Field>
            </>
          )}

          {saveError && (
            <div style={{
              fontSize: textSize.caption, color: colors.danger,
              borderRadius: radius.md, border: `1px solid ${colors.danger}`,
              background: colors.danger + '14', padding: '8px 12px',
            }}>
              {saveError}
            </div>
          )}
        </div>
      )}
    </DetailModal>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  const { colors } = useTheme();
  return (
    <div>
      <div style={{
        fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono,
        textTransform: 'uppercase', letterSpacing: '0.04em', marginBottom: 6,
      }}>
        {label}
      </div>
      {children}
    </div>
  );
}

function MetaGrid({ colors, rows }: {
  colors: ReturnType<typeof useTheme>['colors'];
  rows: [string, string][];
}) {
  return (
    <div style={{ display: 'grid', gridTemplateColumns: 'auto 1fr', gap: '6px 14px' }}>
      {rows.map(([k, v]) => (
        <div key={k} style={{ display: 'contents' }}>
          <span style={{ fontSize: textSize.micro, color: colors.textDim, fontFamily: font.mono, whiteSpace: 'nowrap' }}>
            {k}
          </span>
          <span style={{ fontSize: textSize.caption, color: colors.text, wordBreak: 'break-word', userSelect: 'text' }}>
            {v}
          </span>
        </div>
      ))}
    </div>
  );
}

function inputStyle(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    width: '100%', boxSizing: 'border-box', padding: '7px 9px',
    borderRadius: radius.md, border: `1px solid ${colors.border}`,
    background: colors.inputBg, color: colors.text,
    fontFamily: font.body, fontSize: textSize.caption, outline: 'none',
  };
}

/** The footer's hairline button, expressed as `Button`'s custom properties —
 *  an inline `color`/`border` would beat `.pa-btn:hover` and silently kill the
 *  states this migration exists to add. */
function ghostVars(colors: ReturnType<typeof useTheme>['colors']): CSSProperties {
  return {
    '--pa-btn-fg': colors.textMuted,
    '--pa-btn-fg-hover': colors.text,
    '--pa-btn-border': colors.border,
    '--pa-btn-border-hover': colors.borderHi,
    '--pa-btn-bg-hover': 'transparent',
    '--pa-btn-pad': '6px 14px',
    '--pa-btn-radius': `${radius.md}px`,
    fontFamily: font.body, fontSize: textSize.caption,
  } as CSSProperties;
}

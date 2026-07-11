/**
 * PersonDetailModal (CRM epic slice 2a) — the read-only detail view for a
 * person associated with a project, opened from the Overview People panel.
 *
 * Slice 2a is read-only: it renders the person's typed CRM fields (role /
 * company / email / phone / notes) plus this project's role and association
 * time — ALL carried on the {@link ProjectPerson} the panel already fetched, so
 * the modal makes no extra GET. The one mutation is *disassociate* (DELETE
 * /api/projects/{id}/people/{entity_uuid}, #530), after which it bumps the
 * store's people revision so the decoupled panel refetches, and closes.
 * "Refresh enrichment" (#495 slice 4) mutates nothing here: it copies a
 * prepared prompt and navigates to chat; writes happen only after the user
 * approves the resulting Decision Inbox proposal.
 *
 * Editing the typed fields and showing #499 entity_fields provenance are
 * deliberately deferred to a later slice (each needs its own authoritative-store
 * ruling). Built on the generic DetailModal shell, mirroring GoalDetailModal.
 *
 * PersonDetailModalHost is mounted once at the app root and renders whenever the
 * store's `personDetail` target is set.
 */

import { useState } from 'react';
import { apiFetch } from '../../lib/api';
import { useCommandCenter, navigateToTool } from '../../lib/store';
import { font, radius } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { DetailModal } from '../common/DetailModal';
import type { ProjectPerson } from './types';

function fmtTime(iso: string | null): string {
  if (!iso) return '—';
  const t = Date.parse(iso);
  return Number.isFinite(t) ? new Date(t).toLocaleString() : iso;
}

export function PersonDetailModal({
  projectId,
  person,
  onClose,
}: {
  projectId: string;
  person: ProjectPerson;
  onClose: () => void;
}) {
  const { colors } = useTheme();
  const bumpPeople = useCommandCenter(s => s.bumpPeople);
  const [confirming, setConfirming] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [promptCopied, setPromptCopied] = useState(false);

  // The Enricher (#495 slice 4), prepared-prompt pattern: copy the enrichment
  // request to the clipboard and take the user to chat — the agent runs
  // enrich_person → researches with its web tools → propose_enrichment, and
  // findings wait in the Decision Inbox for approval. Nothing here writes.
  const requestEnrichment = () => {
    const prompt =
      `Refresh enrichment for ${person.display_name}: call enrich_person with ` +
      `person "${person.display_name}", research the enrichable fields with your ` +
      `web tools, then call propose_enrichment so I can review the findings in ` +
      `the Decision Inbox.`;
    navigator.clipboard?.writeText(prompt).catch(() => {});
    setPromptCopied(true);
    navigateToTool('chat');
  };

  const doDisassociate = async () => {
    setRemoving(true);
    setError(null);
    try {
      await apiFetch(
        `/api/projects/${encodeURIComponent(projectId)}/people/${encodeURIComponent(person.entity_uuid)}`,
        { method: 'DELETE' },
      );
      bumpPeople();
      onClose();
    } catch {
      setError("Couldn't remove this person from the project. Please try again.");
      setRemoving(false);
    }
  };

  const badge = person.project_role
    ? { label: person.project_role, color: colors.cyan, bg: colors.cyanSoft }
    : null;

  const footer = confirming ? (
    <>
      <span style={{ flex: 1, fontSize: 12, color: colors.textMuted }}>
        Remove {person.display_name} from this project?
      </span>
      <button onClick={() => setConfirming(false)} disabled={removing} style={ghostBtn(colors)}>
        Keep
      </button>
      <button onClick={doDisassociate} disabled={removing} style={dangerBtn(colors)}>
        {removing ? 'Removing…' : 'Confirm remove'}
      </button>
    </>
  ) : (
    <>
      <button onClick={requestEnrichment} style={ghostBtn(colors)}>
        {promptCopied ? 'Prompt copied — paste it in chat' : 'Refresh enrichment'}
      </button>
      <span style={{ flex: 1 }} />
      <button onClick={() => setConfirming(true)} style={dangerBtn(colors)}>
        Remove from project
      </button>
    </>
  );

  return (
    <DetailModal title={person.display_name} badge={badge} onClose={onClose} footer={footer}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
        <MetaGrid colors={colors} rows={[
          ['Role', person.role || '—'],
          ['Company', person.company || '—'],
          ['Email', person.email || '—'],
          ['Phone', person.phone || '—'],
          ['Project role', person.project_role || '—'],
          ['Associated', fmtTime(person.associated_at)],
          ['Last contact', fmtTime(person.last_contact_at)],
        ]} />

        {person.notes && (
          <div>
            <div style={{
              fontSize: 11, color: colors.textDim, fontFamily: font.mono,
              textTransform: 'uppercase', letterSpacing: '0.04em', marginBottom: 4,
            }}>
              Notes
            </div>
            <div style={{
              fontSize: 13, color: colors.textMuted, lineHeight: 1.5,
              whiteSpace: 'pre-wrap', wordBreak: 'break-word', userSelect: 'text',
            }}>
              {person.notes}
            </div>
          </div>
        )}

        {error && (
          <div style={{
            fontSize: 12, color: colors.danger,
            borderRadius: radius.md, border: `1px solid ${colors.danger}`,
            background: colors.danger + '14', padding: '8px 12px',
          }}>
            {error}
          </div>
        )}
      </div>
    </DetailModal>
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
          <span style={{ fontSize: 11, color: colors.textDim, fontFamily: font.mono, whiteSpace: 'nowrap' }}>
            {k}
          </span>
          <span style={{ fontSize: 12, color: colors.text, wordBreak: 'break-word', userSelect: 'text' }}>
            {v}
          </span>
        </div>
      ))}
    </div>
  );
}

function ghostBtn(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    padding: '6px 14px', borderRadius: radius.md,
    border: `1px solid ${colors.border}`, background: 'none',
    fontFamily: font.body, fontSize: 12, color: colors.textMuted, cursor: 'pointer',
  };
}

function dangerBtn(colors: ReturnType<typeof useTheme>['colors']): React.CSSProperties {
  return {
    padding: '6px 14px', borderRadius: radius.md,
    border: `1px solid ${colors.danger}`, background: colors.danger + '14',
    fontFamily: font.body, fontSize: 12, fontWeight: 500, color: colors.danger, cursor: 'pointer',
  };
}

/** Mounted once at the app root — renders the modal for the active target. */
export function PersonDetailModalHost() {
  const personDetail = useCommandCenter(s => s.personDetail);
  const closePersonDetail = useCommandCenter(s => s.closePersonDetail);
  if (!personDetail) return null;
  return (
    <PersonDetailModal
      projectId={personDetail.projectId}
      person={personDetail.person}
      onClose={closePersonDetail}
    />
  );
}

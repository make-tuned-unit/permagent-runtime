/**
 * World W2 — Garden archive: inspect what is remembered.
 *
 * Real records are the default surface (C4): open one, read its stored text,
 * time and provenance — or an explicit provenance gap — and reopen it in
 * Brain. "Ask about this evidence" prepares a question that cites the record;
 * a field note is saved only to an explicitly chosen project. The fictional
 * evidence exercise is a separate, labelled demonstration.
 */
import { useId, useRef, useState } from 'react';
import { placeSchema } from '../data/placeSchema';
import { recordLinkKey } from '../data/recordLinks';
import { visibleItems, type ActivityRecord, type MemoryRecord, type NoteRecord, type Section } from '../data/types';
import { Button } from '../../../common/Button';
import { BriefComposer } from './BriefComposer';
import { EvidenceSlot } from './EvidenceSlot';
import { FieldNoteEditor } from './FieldNoteEditor';
import { ageLabel, displayName, provenanceLabel } from './format';
import type { PlaceHostProps } from './integration';
import { LinkResultNote } from './LinkResultNote';
import { PlacePanel } from './PlacePanel';
import { RecordListbox, type RecordListboxHandle } from './RecordListbox';
import { usePlaceFrame } from './usePlaceFrame';

const SCHEMA = placeSchema('gallery');
const slot = (id: string) => SCHEMA.evidence.find((s) => s.id === id)!;
let localNoteSeq = 0;
const defaultNewId = () => `world-note-${Date.now().toString(36)}-${(localNoteSeq += 1)}`;

export function GardenArchivePlace(props: PlaceHostProps) {
  const { projects, selectedAgentId, drafts, noteWriter, onRequestSend, onEvent, onClose, returnFocusTo, agentNames } = props;
  const [projectId, setProjectId] = useState<string | null>(props.projectId);
  const { colors, styles, compact, now, snap, openLink, linkResult, stateLine } = usePlaceFrame(props, 'gallery', projectId);
  const [query, setQuery] = useState('');
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [composerOpen, setComposerOpen] = useState(false);
  const listRef = useRef<RecordListboxHandle>(null);
  const projectRef = useRef<HTMLSelectElement>(null);
  const askRef = useRef<HTMLButtonElement>(null);
  const memoriesHeadingRef = useRef<HTMLDivElement>(null);
  const ids = { project: useId(), search: useId() };

  const s = snap.snapshot?.sections ?? {};
  const memories = s.memories as Section<MemoryRecord> | undefined;
  const activity = s.librarian_activity as Section<ActivityRecord> | undefined;
  const notes = s.notes as Section<NoteRecord> | undefined;
  const loaded = memories ? visibleItems(memories, now).items : [];
  const q = query.trim().toLowerCase();
  const shown = q ? loaded.filter((m) => `${m.text} ${m.description ?? ''} ${m.key ?? ''}`.toLowerCase().includes(q)) : loaded;
  const selected = loaded.find((m) => recordLinkKey(m.link) === selectedKey) ?? null;
  const hostName = displayName('librarian', agentNames);
  const placeEmpty = !!snap.snapshot && [memories, notes].every((x) => x?.state === 'empty' || x?.state === 'setup_required') && memories?.state === 'empty';

  const select = (key: string) => {
    setSelectedKey(key);
    onEvent?.({ type: 'record_selected', placeId: 'gallery', recordKey: key });
  };

  const browse = () => {
    if (!listRef.current?.focus()) memoriesHeadingRef.current?.focus();
  };

  const chooseProject = (value: string) => {
    const next = value || null;
    setProjectId(next);
    setSelectedKey(null);
    onEvent?.({ type: 'project_changed', placeId: 'gallery', projectId: next });
  };

  return (
    <PlacePanel
      schema={SCHEMA}
      hostName={hostName}
      stateLine={stateLine([memories, activity, notes])}
      primary={{ id: SCHEMA.primaryAction.id, label: SCHEMA.primaryAction.label, description: SCHEMA.primaryAction.description, onActivate: browse }}
      secondary={[
        { id: 'open_in_brain', label: 'Open in Brain', description: SCHEMA.secondaryActions[0].description, disabled: !selected, disabledReason: 'choose a record first.', onActivate: () => { if (selected) openLink(selected.link); } },
        { id: 'ask_about_evidence', label: 'Ask about this evidence', description: SCHEMA.secondaryActions[1].description, disabled: !selected, disabledReason: 'choose a record first.', expanded: composerOpen, ref: askRef, onActivate: () => setComposerOpen(true) },
        { id: 'try_example', label: 'Try an example', description: SCHEMA.secondaryActions[3].description, demonstration: true, onActivate: () => onEvent?.({ type: 'local_action', placeId: 'gallery', action: 'try_example' }) },
      ]}
      placeEmpty={placeEmpty}
      compact={compact}
      colors={colors}
      styles={styles}
      onClose={onClose}
      onEvent={onEvent}
      returnFocusTo={returnFocusTo}
    >
      <LinkResultNote linkResult={linkResult} styles={styles} />
      <div style={styles.actions} role="group" aria-label="Folio controls">
        <label htmlFor={ids.project} style={styles.label}>Memories from</label>
        <select id={ids.project} ref={projectRef} style={{ ...styles.input, width: 'auto' }} value={projectId ?? ''} onChange={(e) => chooseProject(e.target.value)}>
          <option value="">All memories (Brain)</option>
          {projects.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
        <label htmlFor={ids.search} style={styles.label}>Filter</label>
        <input id={ids.search} type="search" style={{ ...styles.input, width: 'auto', flex: '1 1 auto' }} value={query} placeholder="Filter the records shown" onChange={(e) => setQuery(e.target.value)} />
      </div>
      {q && <p style={styles.note} role="status">{shown.length} of {loaded.length} loaded record(s) match. This filters what is loaded here; Brain searches everything.</p>}
      <BriefComposer
        open={composerOpen && !!selected}
        onClose={() => setComposerOpen(false)}
        triggerRef={askRef}
        placeId="gallery"
        projectId={projectId}
        agentWorldId={selectedAgentId}
        agentName={hostName}
        heading="Ask about this evidence"
        intents={['query']}
        seedEvidence={selected ? [recordLinkKey(selected.link)] : undefined}
        drafts={drafts}
        onRequestSend={onRequestSend}
        onOpenLink={openLink}
        onEvent={onEvent}
        now={now}
        colors={colors}
        styles={styles}
      />
      <div style={styles.body}>
        <div style={styles.column} ref={memoriesHeadingRef} tabIndex={-1}>
          <EvidenceSlot
            slot={slot('memories')} section={memories} loading={snap.loading} now={now} styles={styles} colors={colors} moreWhere="Brain" onRetry={snap.retry}
            renderItems={() => shown.length === 0
              ? <p style={styles.note}>No loaded record matches “{query}”.</p>
              : (
                <RecordListbox
                  ref={listRef}
                  label="Remembered records"
                  items={shown}
                  keyOf={(m) => recordLinkKey(m.link)}
                  selectedKey={selectedKey}
                  onSelect={select}
                  styles={styles}
                  renderOption={(m) => (
                    <>
                      <span style={styles.para}>{m.description ?? m.text}</span>
                      <span style={styles.optionMeta}>{ageLabel(m.timestamp, now)} · {m.provenance.kind === 'none' ? 'no provenance recorded' : m.provenance.kind === 'source_tag' ? `source: ${m.provenance.tag}` : `written as ${m.provenance.origin}`}</span>
                    </>
                  )}
                />
              )}
          />
          <EvidenceSlot
            slot={slot('librarian_activity')} section={activity} loading={snap.loading} now={now} styles={styles} colors={colors} collapsible onRetry={snap.retry}
            renderItems={(items) => (
              <ul style={styles.list}>
                {items.map((a) => (
                  <li key={a.id} style={styles.row} data-record={a.id}>
                    <span style={styles.para}>{a.title}</span>
                    <span style={styles.optionMeta}>{a.actor} · {ageLabel(a.ts, now)}</span>
                  </li>
                ))}
              </ul>
            )}
          />
        </div>
        <div style={styles.column}>
          <section style={styles.slot} aria-label="Inspect record" data-inspect={selected ? selected.id : 'none'}>
            {!selected
              ? <p style={styles.note}>Choose a record to inspect its text, time and provenance.</p>
              : (() => {
                  const prov = provenanceLabel(selected.provenance);
                  return (
                    <div style={styles.column}>
                      <p style={styles.slotTitle}>Stored text</p>
                      <p style={styles.para}>{selected.text}</p>
                      {selected.textClipped && <p style={styles.note}>Shortened here. The full text is in Brain.</p>}
                      {selected.description && <p style={styles.note}>Described as: {selected.description}</p>}
                      <p style={styles.note}>Remembered {ageLabel(selected.timestamp, now)}{selected.projectId ? ` · project ${projects.find((p) => p.id === selected.projectId)?.name ?? selected.projectId}` : ''}.</p>
                      <p style={prov.gap ? styles.warn : styles.para} data-provenance={selected.provenance.kind}>{prov.label}</p>
                      <div style={styles.actions}>
                        <Button colors={colors} flashSuccess={false} onClick={() => { openLink(selected.link); }}>Open in Brain</Button>
                      </div>
                      <FieldNoteEditor
                        placeId="gallery"
                        evidenceRefs={[recordLinkKey(selected.link)]}
                        drafts={drafts}
                        noteWriter={noteWriter}
                        projects={projects}
                        defaultProjectId={projectId}
                        newId={props.newId ?? defaultNewId}
                        onEvent={onEvent}
                        onOpenLink={openLink}
                        now={now}
                        colors={colors}
                        styles={styles}
                      />
                    </div>
                  );
                })()}
          </section>
          <EvidenceSlot
            slot={slot('notes')} section={notes} loading={snap.loading} now={now} styles={styles} colors={colors} collapsible={compact} moreWhere="Projects" onRetry={snap.retry}
            onSetup={() => projectRef.current?.focus()}
            renderItems={(items) => (
              <ul style={styles.list}>
                {items.map((n) => (
                  <li key={n.id} style={styles.row} data-record={n.id}>
                    <span style={styles.para}>{n.title ? `${n.title}: ` : ''}{n.body}</span>
                    <span style={styles.optionMeta}>{ageLabel(n.createdAt, now)}</span>
                    <span><Button colors={colors} variant="bare" flashSuccess={false} onClick={() => { openLink(n.link); }}>Open note</Button></span>
                  </li>
                ))}
              </ul>
            )}
          />
        </div>
      </div>
    </PlacePanel>
  );
}

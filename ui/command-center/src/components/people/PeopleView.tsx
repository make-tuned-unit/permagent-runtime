/**
 * People — a first-class tab, same header as every other top-level view.
 *
 * Until this existed, People was a pill toggle above Projects. That made the
 * Projects header inconsistent with Build/Grow/Automate (which wear ViewHeader
 * alone) and hid the directory behind a sub-view. Graph is the default: you
 * sit at the center, people cluster by project around you, and an edge runs
 * from you to each contact. Shared-project edges between people are the first
 * glimpse of connections that do not run through you. List is the original
 * directory, kept as the other mode of this one tab.
 */

import { useEffect, useState } from 'react';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useCommandCenter } from '../../lib/store';
import { apiFetch } from '../../lib/api';
import { ViewHeader } from '../common/ViewHeader';
import { PersonDetailModal } from '../projects/PersonDetailModal';
import type { Person } from '../projects/types';
import { PeopleDirectory } from './PeopleDirectory';
import { PeopleGraph } from './PeopleGraphCanvas';
import { matchPendingPerson } from './personNavigation';

type PeopleMode = 'graph' | 'list';
const MODE_KEY = 'permagent-people-mode';

function readMode(): PeopleMode {
  try {
    return localStorage.getItem(MODE_KEY) === 'list' ? 'list' : 'graph';
  } catch {
    return 'graph';
  }
}

export function PeopleView() {
  const { gradient } = useTheme();
  const [mode, setMode] = useState<PeopleMode>(readMode);
  const personDetail = useCommandCenter(s => s.personDetail);
  const closePersonDetail = useCommandCenter(s => s.closePersonDetail);
  const openPersonDetail = useCommandCenter(s => s.openPersonDetail);
  const bumpPeople = useCommandCenter(s => s.bumpPeople);
  const pendingPersonNavigation = useCommandCenter(s => s.pendingPersonNavigation);
  const setPendingPersonNavigation = useCommandCenter(s => s.setPendingPersonNavigation);
  const selected = personDetail && personDetail.projectId == null ? personDetail : null;

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const res = await apiFetch<{ imported: number }>('/api/people/calendar/import', { method: 'POST' });
        if (!cancelled && res.imported > 0) bumpPeople();
      } catch { /* Calendar permission is optional */ }
    })();
    return () => { cancelled = true; };
  }, [bumpPeople]);

  useEffect(() => {
    if (!pendingPersonNavigation) return;
    let cancelled = false;
    (async () => {
      try {
        const people = await apiFetch<Person[]>('/api/people');
        if (cancelled) return;
        const match = Array.isArray(people)
          ? matchPendingPerson(people, pendingPersonNavigation)
          : null;
        if (match) openPersonDetail(null, match);
      } catch { /* drop — a missing directory must not loop */ }
      if (!cancelled) setPendingPersonNavigation(null);
    })();
    return () => { cancelled = true; };
  }, [pendingPersonNavigation, openPersonDetail, setPendingPersonNavigation]);

  const switchMode = (next: PeopleMode) => {
    try { localStorage.setItem(MODE_KEY, next); } catch { /* ignore */ }
    setMode(next);
  };

  return (
    <div style={{
      width: '100%',
      height: '100%',
      display: 'flex',
      flexDirection: 'column',
      background: gradient.workspace,
    }}>
      <ViewHeader
        title="People"
        subtitle="Quiet contacts fade. Follow-ups live on the person and on Home."
        afterTitle={<ModeToggle mode={mode} onChange={switchMode} />}
      />
      <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
        <div style={{ flex: 1, minWidth: 0, overflow: mode === 'list' ? 'auto' : 'hidden' }}>
          {mode === 'graph' ? <PeopleGraph /> : <PeopleDirectory />}
        </div>
        {selected && (
          <PersonDetailModal
            key={selected.person.entity_uuid}
            variant="inline"
            projectId={null}
            person={selected.person}
            association={selected.association}
            onClose={closePersonDetail}
          />
        )}
      </div>
    </div>
  );
}

function ModeToggle({ mode, onChange }: { mode: PeopleMode; onChange: (m: PeopleMode) => void }) {
  const { colors } = useTheme();
  const tabs: { key: PeopleMode; label: string }[] = [
    { key: 'graph', label: 'Graph' },
    { key: 'list', label: 'List' },
  ];
  return (
    <div style={{
      display: 'inline-flex', gap: 2, padding: 2, borderRadius: 8,
      background: 'rgba(255,255,255,0.04)', border: `1px solid ${colors.border}`,
    }}>
      {tabs.map(t => {
        const active = mode === t.key;
        return (
          <button
            key={t.key}
            onClick={() => onChange(t.key)}
            style={{
              padding: '4px 12px', borderRadius: 6, cursor: 'pointer', border: 'none',
              background: active ? colors.cyanSoft : 'transparent',
              color: active ? colors.cyan : colors.textMuted,
              fontFamily: font.body, fontSize: 12, fontWeight: active ? 600 : 500,
              transition: 'all 150ms',
            }}
          >
            {t.label}
          </button>
        );
      })}
    </div>
  );
}

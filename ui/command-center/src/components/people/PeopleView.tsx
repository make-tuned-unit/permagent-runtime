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

import { useEffect, useState, type CSSProperties } from 'react';
import { font, radius, type, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useCommandCenter } from '../../lib/store';
import { apiFetch } from '../../lib/api';
import { ViewHeader } from '../common/ViewHeader';
import { Button } from '../common/Button';
import { calendarImportLine, type CalendarImportPhase } from './calendarImport';
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
  const patchPersonDetail = useCommandCenter(s => s.patchPersonDetail);
  const bumpPeople = useCommandCenter(s => s.bumpPeople);
  const peopleRev = useCommandCenter(s => s.peopleRev);
  const pendingPersonNavigation = useCommandCenter(s => s.pendingPersonNavigation);
  const setPendingPersonNavigation = useCommandCenter(s => s.setPendingPersonNavigation);
  const selected = personDetail && personDetail.projectId == null ? personDetail : null;

  // Reading the user's calendar is not a background detail — it is personal
  // data pulled without being asked, on every mount. It now says it happened,
  // and a failure says so instead of being swallowed.
  const [calendar, setCalendar] = useState<CalendarImportPhase>({ phase: 'importing' });
  const [calendarRun, setCalendarRun] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setCalendar({ phase: 'importing' });
    (async () => {
      try {
        const res = await apiFetch<{ imported: number }>('/api/people/calendar/import', { method: 'POST' });
        if (cancelled) return;
        const imported = Number(res?.imported) || 0;
        setCalendar({ phase: 'done', imported, at: Date.now() });
        if (imported > 0) bumpPeople();
      } catch (e) {
        if (cancelled) return;
        setCalendar({
          phase: 'failed',
          message: e instanceof Error ? e.message : 'the daemon did not answer',
        });
      }
    })();
    return () => { cancelled = true; };
  }, [bumpPeople, calendarRun]);

  useEffect(() => {
    if (!selected) return;
    let cancelled = false;
    (async () => {
      try {
        const people = await apiFetch<Person[]>('/api/people');
        if (cancelled || !Array.isArray(people)) return;
        const fresh = people.find(p => p.entity_uuid === selected.person.entity_uuid);
        if (fresh) patchPersonDetail(fresh);
      } catch { /* drop — a failed refresh must not loop */ }
    })();
    return () => { cancelled = true; };
  }, [peopleRev]); // eslint-disable-line react-hooks/exhaustive-deps

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
        actions={<CalendarImportNote state={calendar} onRetry={() => setCalendarRun(n => n + 1)} />}
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

/**
 * One quiet line for the import that runs behind this tab. It is a caption,
 * not a banner: the acknowledgment belongs on screen, but it is not news.
 */
function CalendarImportNote({
  state,
  onRetry,
}: {
  state: CalendarImportPhase;
  onRetry: () => void;
}) {
  const { colors } = useTheme();
  const line = calendarImportLine(state);
  return (
    <div style={{ display: 'inline-flex', alignItems: 'center', gap: 8 }}>
      <span
        data-testid="people-calendar-note"
        title={line.title}
        style={{
          ...type.micro,
          color: line.tone === 'warning' ? colors.warning : colors.textMuted,
          cursor: line.title ? 'help' : undefined,
        }}
      >
        {line.text}
      </span>
      {line.retry && (
        <Button colors={colors} type="button" flashSuccess={false} onClick={onRetry}>
          Retry
        </Button>
      )}
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
      display: 'inline-flex', gap: 2, padding: 2, borderRadius: radius.md,
      background: 'rgba(255,255,255,0.04)', border: `1px solid ${colors.border}`,
    }}>
      {tabs.map(t => {
        const active = mode === t.key;
        return (
          <Button
            key={t.key}
            colors={colors}
            variant="bare"
            onClick={() => onChange(t.key)}
            style={{
              '--pa-btn-bg': active ? colors.cyanSoft : 'transparent',
              '--pa-btn-fg': active ? colors.cyan : colors.textMuted,
              '--pa-btn-border': 'transparent',
              '--pa-btn-bg-hover': active ? colors.cyanSoft : colors.surfaceHi,
              '--pa-btn-fg-hover': active ? colors.cyan : colors.text,
              '--pa-btn-bg-active': active ? colors.cyanSoft : colors.surface,
              // One pixel off each edge pays for `.pa-btn`'s hairline border,
              // so the segmented control keeps the height it has today.
              '--pa-btn-pad': '3px 11px',
              '--pa-btn-radius': `${radius.sm}px`,
              '--pa-btn-weight': active ? 600 : 500,
              fontFamily: font.body, fontSize: textSize.caption,
            } as CSSProperties}
          >
            {t.label}
          </Button>
        );
      })}
    </div>
  );
}

/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import { RunDetail, findingsReportNotice, FindingsPanel, BulkConfirmDialog, type Finding, type BulkSweepSummary } from './AutomateView';
import { idleJob } from '../../hooks/useLongRunningJob';
import { apiFetch } from '../../lib/api';
import { REGENERABLE_ACK, REVIEW_ACK } from './findingCategories';

vi.mock('../../lib/api', async importOriginal => ({
  ...await importOriginal<typeof import('../../lib/api')>(), apiFetch: vi.fn(),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const gb = 1024 ** 3;
const rows = (category: string, count: number, bytes: number): Finding[] => Array.from({ length: count }, (_, i) => ({
  id: `${category}-${i}`, path: `/fixture/${category}/${i}`, type: 'dev_cache', category,
  size_bytes: bytes / count, age_days: null, recommendation: category, consequence: 'Fixture consequence',
  action_taken: null, actioned_at: null, size_recovered_bytes: null,
}));
const ledger = [
  ...rows('in_use', 12, 30 * gb), ...rows('review_before_removing', 5, 2.4 * gb),
  ...rows('regenerable_costly', 4, 1.1 * gb), ...rows('managed_by_macos', 1, 1.5 * gb),
];
let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.clearAllMocks();
  container = document.createElement('div'); document.body.appendChild(container); root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

it('acceptance 7: September 15 hero and dialog account for all 22 pending items without selecting 35 GB', async () => {
  vi.mocked(apiFetch).mockResolvedValue({ findings: ledger, availability: ledger.map(f => ({
    finding_id: f.id, category: f.category, eligible: !['in_use', 'managed_by_macos'].includes(f.category!),
    reason: 'Fixture consequence', protection: 'probe-only',
  })) });
  await act(async () => root.render(<FindingsPanel runId="09-15" findings={ledger} actionInFlight={null}
    onAction={vi.fn()} onBulkAction={vi.fn()} totalRecovered={0} allActioned={false} lifetime={null} />));
  expect(container.textContent).toContain('35.0 GB found (estimate)');
  const open = [...container.querySelectorAll('button')].find(b => b.textContent === 'Review cleanup options');
  expect(open).toBeTruthy();
  await act(async () => open!.click());
  expect(container.textContent).toContain('Selected — 0 items · 0 B estimated');
  expect(container.textContent).toContain('Optional — rebuildable — 4 items · 1.1 GB estimated');
  expect(container.textContent).toContain('Optional — requiring review — 5 items · 2.4 GB estimated');
  expect(container.textContent).toContain('Excluded — will not be removed — 13 items · 31.5 GB estimated');
  // Exactly once: count leaf elements whose text is exactly the path (textContent
  // concatenates the path and the size with no separator, so substring counts lie).
  const leaves = [...container.querySelectorAll('*')].filter(el => el.childElementCount === 0);
  for (const f of ledger) expect(leaves.filter(el => el.textContent === f.path).length).toBe(1);
  for (const button of container.querySelectorAll('button')) expect(button.textContent).not.toContain('35.0 GB');
  const confirm = [...container.querySelectorAll('button')].find(b => b.textContent === 'Move 0 items to Trash — 0 B');
  expect(confirm?.disabled).toBe(true);
  expect(container.textContent).toContain(REGENERABLE_ACK);
  expect(container.textContent).toContain(REVIEW_ACK);
});

it('acceptance 2 and 7: both acknowledgments select only their named buckets with one confirmation', async () => {
  const confirm = vi.fn();
  await act(async () => root.render(<BulkConfirmDialog pending={ledger} includeRegenerable includeReview
    onToggleRegenerable={vi.fn()} onToggleReview={vi.fn()} onCancel={vi.fn()} onConfirm={confirm}
    sweep={idleJob<BulkSweepSummary>()} />));
  const move = [...container.querySelectorAll('button')].find(b => b.textContent === 'Move 9 items to Trash — 3.5 GB');
  expect(move?.disabled).toBe(false);
  await act(async () => move!.click());
  expect(confirm).toHaveBeenCalledOnce();
  expect(confirm.mock.calls[0][0]).toHaveLength(9);
});

it('acceptance 3 and 5: availability checks and failed checks disable confirmation', async () => {
  for (const props of [{ checking: true }, { checkError: 'Activity check timed out' }]) {
    await act(async () => root.render(<BulkConfirmDialog pending={ledger} includeRegenerable includeReview
      onToggleRegenerable={vi.fn()} onToggleReview={vi.fn()} onCancel={vi.fn()} onConfirm={vi.fn()}
      sweep={idleJob<BulkSweepSummary>()} {...props} />));
    const buttons = [...container.querySelectorAll('button')];
    expect(buttons[buttons.length - 1]?.disabled).toBe(true);
    expect(container.textContent).toContain(props.checking ? 'Checking availability…' : 'Activity check timed out');
  }
});

it('acceptance 7: a partial sweep reports moved and skipped counts without claiming freed bytes', async () => {
  const safe = rows('safe_to_remove', 2, gb);
  vi.mocked(apiFetch).mockResolvedValue({ findings: safe, availability: safe.map(f => ({
    finding_id: f.id, category: f.category, eligible: true, reason: 'Available', protection: 'probe-only',
  })) });
  const onBulkAction = vi.fn().mockResolvedValue({ cleaned: 1, failed: 1, bytes: gb / 2 });
  await act(async () => root.render(<FindingsPanel runId="partial" findings={safe} actionInFlight={null}
    onAction={vi.fn()} onBulkAction={onBulkAction} totalRecovered={0} allActioned={false} lifetime={null} />));
  const open = [...container.querySelectorAll('button')].find(b => b.textContent === 'Review cleanup — 1.0 GB selected');
  await act(async () => open!.click());
  const move = [...container.querySelectorAll('button')].find(b => b.textContent === 'Move 2 items to Trash — 1.0 GB');
  await act(async () => move!.click());
  expect(onBulkAction).toHaveBeenCalledWith(safe.map(f => f.id), { include_regenerable: false, include_review: false });
  expect(container.textContent).toContain('Moved 1 to Trash · 1 could not be removed');
  expect(container.textContent).not.toContain('recovered');
  expect(container.textContent).not.toContain('freed');
});


it('acceptance 8: September 22 ok/error without a ledger never claims an empty successful scan', () => {
  for (const status of ['ok', 'error'] as const) {
    expect(findingsReportNotice({ ledger_state: 'missing', findings: [] }, status))
      .toBe('Report incomplete — cleanup findings could not be saved.');
  }
  expect(findingsReportNotice({ ledger_state: 'unverified', findings: [] }, 'ok'))
    .toBe('Report incomplete — cleanup findings could not be saved.');
  expect(findingsReportNotice({ ledger_state: 'valid', findings: [] }, 'ok'))
    .toBe('Scan complete — no cleanup candidates found.');
  expect(findingsReportNotice({ ledger_state: 'valid', findings: [] }, 'error'))
    .toBe('Report incomplete — cleanup findings could not be saved.');
});

it('acceptance 8: missing September 22 ledger retains September 15 report with its original date', async () => {
  const previousDate = '2026-09-15T09:00:00Z';
  vi.mocked(apiFetch).mockImplementation(async path => {
    const url = String(path);
    if (url === '/automation/run/09-22/findings') return { ledger_state: 'missing', findings: [] };
    if (url === '/automation/run/09-15/findings') return {
      ledger_state: 'valid', findings: ledger, created_at: previousDate,
    };
    if (url.startsWith('/schedule/storage/sessions')) return [{ id: '09-15', createdAt: previousDate }];
    if (url === '/api/sessions/09-15') return {
      conversation: [{ role: 'assistant', content: [{ type: 'text', text: 'Previous storage report' }] }],
    };
    return {};
  });
  for (const lastStatus of ['ok', 'error'] as const) {
    await act(async () => root.render(<RunDetail key={lastStatus} run={{
      id: '09-22', jobId: 'storage', name: 'Storage Insights', createdAt: '2026-09-22T09:00:00Z',
      workingDir: '/', scheduleId: 'storage', messageCount: 2, totalTokens: 10,
      inputTokens: 5, outputTokens: 5,
    }} displayName="Storage Insights" lastStatus={lastStatus} />));
    expect(container.textContent).toContain('Report incomplete — cleanup findings could not be saved.');
    expect(container.textContent).not.toContain('Scan complete — no cleanup candidates found.');
    expect(container.textContent).toContain('Previous storage report');
    expect(container.textContent).toContain('Previous valid report — ' + previousDate);
    expect(container.querySelector(`time[datetime="${previousDate}"]`)).not.toBeNull();
  }
});

it('acceptance 2 and 7: confirms exact idle profile paths and excludes busy and unsupported bytes', async () => {
  const target: Finding = { ...rows('regenerable_costly', 1, 30 * gb)[0], path: '/fixture/target' };
  const profiles = [
    { path: '/fixture/target/debug', bytes: gb, status: 'eligible', reason: 'Available build outputs' },
    { path: '/fixture/target/release', bytes: 2 * gb, status: 'busy', reason: 'Profile in use: active process' },
    { path: '/fixture/target/custom', bytes: 3 * gb, status: 'unsupported', reason: 'Unsupported layout: missing .cargo-build-lock' },
  ].map(p => ({ ...p, moved: 0, skipped: 0, failed: 0, moved_bytes: 0 }));
  vi.mocked(apiFetch).mockResolvedValue({ findings: [target], availability: [{
    finding_id: target.id, category: target.category, eligible: true, reason: 'Available build outputs',
    protection: 'cargo-profile-locks', profiles,
  }] });
  const onBulkAction = vi.fn().mockResolvedValue({ cleaned: 1, failed: 1, bytes: gb, profiles: [
    { ...profiles[0], status: 'moved', moved: 1, moved_bytes: gb },
    { ...profiles[1], skipped: 1 }, profiles[2],
  ] });
  await act(async () => root.render(<FindingsPanel runId="profiles" findings={[target]} actionInFlight={null}
    onAction={vi.fn()} onBulkAction={onBulkAction} totalRecovered={0} allActioned={false} lifetime={null} />));
  await act(async () => [...container.querySelectorAll('button')].find(b => b.textContent === 'Review cleanup options')!.click());
  expect(container.textContent).toContain('Build outputs are cleaned per profile. Profiles in use are skipped. Profile folders and Cargo lock files remain. Cleaned outputs will need rebuilding.');
  expect(container.textContent).toContain('Eligible profile contents — 1.0 GB estimated');
  expect(container.textContent).toContain('Busy profile contents — 2.0 GB estimated');
  expect(container.textContent).toContain('Unsupported profile contents — 3.0 GB estimated');
  await act(async () => (container.querySelector('input[type="checkbox"]') as HTMLInputElement).click());
  const confirm = [...container.querySelectorAll('button')].find(b => b.textContent === 'Move 1 items to Trash — 1.0 GB');
  expect(confirm).toBeTruthy();
  await act(async () => confirm!.click());
  expect(onBulkAction).toHaveBeenCalledWith([target.id], {
    include_regenerable: true, include_review: false,
    profile_paths: { [target.id]: ['/fixture/target/debug'] },
  });
  expect(container.textContent).toContain('1 moved · 0 skipped · 0 failed · 1.0 GB moved');
  expect(container.textContent).toContain('0 moved · 1 skipped · 0 failed');
});

/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import { RunDetail, findingsReportNotice, FindingsPanel, FindingRow, BulkConfirmDialog, type Finding, type BulkSweepSummary } from './AutomateView';
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

const savedRun = [
  { id: 'xcode', path: '/fixture/xcode/DerivedData', type: 'build_artifact', category: 'safe_to_remove',
    size_bytes: 4 * gb, age_days: 3, recommendation: 'Safe to remove', consequence: 'the next Xcode build is a full rebuild',
    action_taken: null, actioned_at: null, size_recovered_bytes: null },
  { id: 'npm', path: '/fixture/.npm', type: 'app_cache', category: 'regenerable_costly',
    size_bytes: 2 * gb, age_days: 1, recommendation: 'Regenerable but costly', consequence: 're-downloads the next time npm is used',
    action_taken: null, actioned_at: null, size_recovered_bytes: null },
  { id: 'live', path: '/fixture/proj/node_modules', type: 'dev_cache', category: 'in_use',
    size_bytes: 8 * gb, age_days: 0, recommendation: 'In use — do not remove', consequence: 'permagentd is using this directory',
    action_taken: null, actioned_at: null, size_recovered_bytes: null, blocked_reason: null },
  { id: 'nested', path: '/fixture/proj/node_modules/pkg/node_modules', type: 'dev_cache', category: 'safe_to_remove',
    size_bytes: gb, age_days: 0, recommendation: 'Safe to remove', consequence: 'npm install re-fetches this package tree',
    action_taken: null, actioned_at: null, size_recovered_bytes: null, contained_in: 'live' },
  { id: 'download', path: '/fixture/Downloads/archive.iso', type: 'large_file', category: 'review_before_removing',
    size_bytes: 500 * 1024 * 1024, age_days: 40, recommendation: 'Review before removing', consequence: 'Needs review',
    action_taken: null, actioned_at: null, size_recovered_bytes: null },
] satisfies Finding[];

it('a prose report with a saved ledger renders selectable cleanup and keeps protected items out', async () => {
  vi.mocked(apiFetch).mockImplementation(async path => {
    const url = String(path);
    if (url === '/automation/recovery/total') return { total_recovered_bytes: 0, runs_with_recovery: 0, items_trashed: 0 };
    if (url === '/api/sessions/saved') return {
      conversation: [{ role: 'assistant', content: [{ type: 'text', text: '**Your disk has 6.9 GB of cleanup opportunities across 13 items.**' }] }],
    };
    if (url === '/automation/run/saved/findings') return {
      ledger_state: 'valid',
      findings: savedRun,
      rejected: [{ id: 'outside', path: '/fixture/not-a-cache', reason: 'Path is outside the scanner cleanup scope' }],
      created_at: '2026-09-27T12:00:00Z',
    };
    if (url.includes('bulk-preview')) return {
      findings: savedRun,
      availability: savedRun.map(f => ({
        finding_id: f.id, category: f.category, eligible: !['in_use', 'managed_by_macos'].includes(f.category!) && !f.contained_in && !f.blocked_reason,
        reason: f.blocked_reason || f.consequence, protection: 'probe-only',
      })),
    };
    return {};
  });
  await act(async () => root.render(<RunDetail run={{
    id: 'saved', jobId: 'storage', name: 'Storage Insights', createdAt: '2026-09-27T12:00:00Z',
    workingDir: '/', scheduleId: 'storage', messageCount: 2, totalTokens: 10, inputTokens: 5, outputTokens: 5,
  }} displayName="Storage Insights" lastStatus="ok" />));
  expect(container.textContent).not.toContain('Report incomplete — cleanup findings could not be saved.');
  expect(container.textContent).not.toContain('6.9 GB of cleanup opportunities');
  expect(container.textContent).toContain('Not saved — /fixture/not-a-cache: Path is outside the scanner cleanup scope');
  const open = [...container.querySelectorAll('button')].find(b => b.textContent?.startsWith('Review cleanup'));
  expect(open).toBeTruthy();
  await act(async () => open!.click());
  expect(container.textContent).toContain('/fixture/xcode/DerivedData');
  expect(container.textContent).toContain('/fixture/.npm');
  expect(container.textContent).toContain('/fixture/proj/node_modules');
  expect(container.textContent).toContain('/fixture/Downloads/archive.iso');
  expect(container.textContent).toContain('14.5 GB found (estimate)');
  expect(container.textContent).toContain('2 items · 8.0 GB');
  expect(container.textContent).toContain('Selected — 1 items');
  expect(container.textContent).toContain('Optional — rebuildable — 1 items');
  expect(container.textContent).toContain('Optional — requiring review — 1 items');
  expect(container.textContent).toContain('Excluded — will not be removed — 2 items · 8.0 GB estimated');
  expect(container.textContent).toContain('Included in another finding; not counted separately');
  const move = [...container.querySelectorAll('button')].find(b => b.textContent === 'Move 1 items to Trash — 4.0 GB');
  expect(move?.disabled).toBe(false);
});

it('saved findings stay selectable when the run status is an error', async () => {
  vi.mocked(apiFetch).mockImplementation(async path => {
    const url = String(path);
    if (url === '/automation/run/saved/findings') return { ledger_state: 'valid', findings: [savedRun[0]], created_at: '2026-09-27T12:00:00Z' };
    if (url === '/api/sessions/saved') return { conversation: [{ role: 'assistant', content: [{ type: 'text', text: 'Partial narration' }] }] };
    return {};
  });
  await act(async () => root.render(<RunDetail run={{
    id: 'saved', jobId: 'storage', name: 'Storage Insights', createdAt: '2026-09-27T12:00:00Z',
    workingDir: '/', scheduleId: 'storage', messageCount: 2, totalTokens: 10, inputTokens: 5, outputTokens: 5,
  }} displayName="Storage Insights" lastStatus="error" />));
  expect(container.textContent).not.toContain('Report incomplete — cleanup findings could not be saved.');
  const open = [...container.querySelectorAll('button')].find(b => b.textContent?.startsWith('Review cleanup'));
  await act(async () => open!.click());
  expect(container.textContent).toContain('/fixture/xcode/DerivedData');
});

it('a failed delete shows the reason and retry sends the action again', async () => {
  const row = { ...savedRun[0], action_taken: 'error', error_message: 'Native Trash failed' };
  const onAction = vi.fn();
  await act(async () => root.render(<FindingRow finding={row} loading={false} onAction={onAction} />));
  expect(container.textContent).toContain('Native Trash failed');
  const retry = [...container.querySelectorAll('button')].find(b => b.textContent === 'Retry');
  expect(retry).toBeTruthy();
  await act(async () => retry!.click());
  expect(onAction).toHaveBeenCalledWith('trash');
});

it('a failed sweep stays on screen and can be tried again without dropping blocked items', async () => {
  const safe = [savedRun[0]];
  vi.mocked(apiFetch).mockResolvedValue({ findings: safe, availability: [{
    finding_id: safe[0].id, category: safe[0].category, eligible: true, reason: 'Available', protection: 'probe-only',
  }] });
  const onBulkAction = vi.fn()
    .mockRejectedValueOnce(new Error('Trash is unavailable'))
    .mockResolvedValueOnce({ cleaned: 1, failed: 0, bytes: safe[0].size_bytes });
  await act(async () => root.render(<FindingsPanel runId="retry" findings={safe} actionInFlight={null}
    onAction={vi.fn()} onBulkAction={onBulkAction} totalRecovered={0} allActioned={false} lifetime={null} />));
  await act(async () => [...container.querySelectorAll('button')].find(b => b.textContent === 'Review cleanup — 4.0 GB selected')!.click());
  await act(async () => [...container.querySelectorAll('button')].find(b => b.textContent === 'Move 1 items to Trash — 4.0 GB')!.click());
  expect(container.textContent).toContain('Trash is unavailable');
  const again = [...container.querySelectorAll('button')].find(b => b.textContent === 'Try again');
  expect(again).toBeTruthy();
  await act(async () => again!.click());
  expect(onBulkAction).toHaveBeenCalledTimes(2);
  expect(container.textContent).toContain('Moved 1 to Trash');
});


it('nested and blocked findings remain visible without independent Trash or Retry actions', async () => {
  for (const row of [
    { ...savedRun[3], action_taken: null },
    { ...savedRun[0], action_taken: 'error', blocked_reason: 'Contains a protected checkout' },
  ]) {
    const onAction = vi.fn();
    await act(async () => root.render(<FindingRow finding={row} loading={false} onAction={onAction} />));
    expect(container.textContent).toContain(row.path.split('/').pop()!);
    expect(container.textContent).toContain(row.blocked_reason || 'Included in another finding');
    expect([...container.querySelectorAll('button')].map(b => b.textContent)).toEqual(['Keep']);
    await act(async () => container.querySelector('button')!.click());
    expect(onAction).toHaveBeenCalledWith('keep');
  }
});

it('both acknowledgments include optional rows but never nested or blocked findings', async () => {
  const blocked = { ...savedRun[0], id: 'blocked', blocked_reason: 'Contains a protected checkout' };
  const onConfirm = vi.fn();
  await act(async () => root.render(<BulkConfirmDialog pending={[...savedRun, blocked]} includeRegenerable includeReview
    onToggleRegenerable={vi.fn()} onToggleReview={vi.fn()} onCancel={vi.fn()} onConfirm={onConfirm}
    sweep={idleJob<BulkSweepSummary>()} />));
  expect(container.textContent).toContain('Contains a protected checkout');
  const move = [...container.querySelectorAll('button')].find(b => b.textContent === 'Move 3 items to Trash — 6.5 GB');
  expect(move?.disabled).toBe(false);
  await act(async () => move!.click());
  expect(onConfirm.mock.calls[0][0].map((f: Finding) => f.id)).toEqual(['xcode', 'npm', 'download']);
});


it('an all-rejected validated scan shows reasons without empty-success or Trash controls', async () => {
  const rejected = [
    { id: 'outside', path: '/fixture/outside', reason: 'Outside cleanup scope' },
    { id: 'invalid', reason: 'Invalid path' },
  ];
  vi.mocked(apiFetch).mockImplementation(async path => {
    const url = String(path);
    if (url === '/automation/run/rejected/findings') return { ledger_state: 'valid', findings: [], rejected };
    if (url === '/api/sessions/rejected') return { conversation: [] };
    if (url === '/automation/recovery/total') return { total_recovered_bytes: 0, runs_with_recovery: 0, items_trashed: 0 };
    throw new Error(`Unexpected API request: ${url}`);
  });
  for (const lastStatus of ['ok', 'error'] as const) {
    await act(async () => root.render(<RunDetail key={lastStatus} run={{
      id: 'rejected', jobId: 'storage', name: 'Storage Insights', createdAt: '2026-09-27T12:00:00Z',
      workingDir: '/', scheduleId: 'storage', messageCount: 0, totalTokens: 0, inputTokens: 0, outputTokens: 0,
    }} displayName="Storage Insights" lastStatus={lastStatus} />));
    expect(container.textContent).toContain('No actionable cleanup findings — scanned items were rejected.');
    expect(container.textContent).toContain('Not saved — /fixture/outside: Outside cleanup scope');
    expect(container.textContent).toContain('Not saved — invalid: Invalid path');
    expect(container.textContent).not.toContain('no cleanup candidates found');
    expect(container.textContent).not.toContain('Report incomplete');
    expect(container.textContent).not.toContain('No report was captured');
    expect([...container.querySelectorAll('button')].some(b => /Trash|Review cleanup/.test(b.textContent || ''))).toBe(false);
    expect(container.textContent).not.toContain('Recover saved scan results');
  }
});

const recoveredFinding: Finding = {
  id: 'app', path: '/fixture/Library/Caches/App', type: 'app_cache', category: 'safe_to_remove',
  size_bytes: 100, age_days: null, recommendation: 'Safe to remove', consequence: 'caches rebuild',
  action_taken: null, actioned_at: null, size_recovered_bytes: null,
};

it('offers recover only for a missing ledger, then reloads the saved scan without sending rows', async () => {
  let releasePost: () => void = () => {};
  const postGate = new Promise<void>(resolve => { releasePost = resolve; });
  let findingsReads = 0;
  const calls: Array<{ url: string; method?: string; body?: BodyInit | null }> = [];
  vi.mocked(apiFetch).mockImplementation(async (path, options) => {
    const url = String(path);
    calls.push({ url, method: options?.method, body: options?.body });
    if (url === '/api/sessions/saved-run') return { conversation: [{ role: 'assistant', content: [{ type: 'text', text: '6.9 GB across 13 items.' }] }] };
    if (url === '/automation/recovery/total') return { total_recovered_bytes: 0, runs_with_recovery: 0, items_trashed: 0 };
    if (url === '/automation/run/saved-run/recover-findings') {
      await postGate;
      return {};
    }
    if (url === '/automation/run/saved-run/findings') {
      findingsReads += 1;
      if (findingsReads === 1) return { ledger_state: 'missing', findings: [] };
      return {
        ledger_state: 'valid', findings: [recoveredFinding], rejected: [],
        created_at: '2026-09-10T00:00:00Z', recovered_at: '2026-09-27T23:00:00Z',
      };
    }
    if (url.includes('/bulk-preview')) return { findings: [recoveredFinding], availability: [{ finding_id: recoveredFinding.id, category: recoveredFinding.category, eligible: true }] };
    if (url.startsWith('/schedule/')) return [];
    throw new Error(`Unexpected API request: ${url}`);
  });
  const run = {
    id: 'saved-run', jobId: 'storage', name: 'Storage Insights', createdAt: '2026-09-22T09:00:00Z',
    workingDir: '/', scheduleId: 'storage', messageCount: 2, totalTokens: 10, inputTokens: 5, outputTokens: 5,
  };
  await act(async () => root.render(<RunDetail run={run} displayName="Storage Insights" starterId="storage_insights" lastStatus="error" />));
  expect(container.textContent).toContain('Report incomplete — cleanup findings could not be saved.');
  const button = [...container.querySelectorAll('button')].find(b => b.textContent === 'Recover saved scan results');
  expect(button).toBeTruthy();
  await act(async () => { button!.click(); });
  expect(container.textContent).toContain('Recovering saved scan results…');
  await act(async () => { releasePost(); await postGate; });
  await act(async () => [...container.querySelectorAll('button')].find(b => b.textContent === 'Review cleanup — 100 B selected')!.click());
  expect(container.textContent).toContain('/fixture/Library/Caches/App');
  expect(container.textContent).toContain('Recovered saved scan results at 2026-09-27T23:00:00Z');
  expect(container.textContent).toContain('Original scan 2026-09-10T00:00:00Z');
  expect(container.textContent).toContain('Sizes are the saved estimates.');
  expect(container.textContent).not.toContain('Report incomplete — cleanup findings could not be saved.');
  expect(container.textContent).not.toContain('no cleanup candidates found');
  const post = calls.find(call => call.url === '/automation/run/saved-run/recover-findings');
  expect(post?.method).toBe('POST');
  expect(post?.body).toBeUndefined();
});

it('a failed recovery stays incomplete and keeps the previous valid report', async () => {
  const previousDate = '2026-09-15T09:00:00Z';
  vi.mocked(apiFetch).mockImplementation(async path => {
    const url = String(path);
    if (url === '/automation/run/saved-run/recover-findings') {
      throw new Error('Saved scan results could not be recovered. incomplete scan findings JSON');
    }
    if (url === '/automation/run/saved-run/findings') return { ledger_state: 'missing', findings: [] };
    if (url === '/automation/run/09-15/findings') return {
      ledger_state: 'valid',
      findings: [{ ...recoveredFinding, id: 'old', path: '/fixture/previous-cache' }],
      created_at: previousDate,
    };
    if (url.startsWith('/schedule/storage/sessions')) return [{ id: '09-15', createdAt: previousDate }];
    if (url === '/api/sessions/09-15') return {
      conversation: [{ role: 'assistant', content: [{ type: 'text', text: 'Previous storage report' }] }],
    };
    if (url === '/api/sessions/saved-run') return { conversation: [] };
    if (url === '/automation/recovery/total') return { total_recovered_bytes: 0, runs_with_recovery: 0, items_trashed: 0 };
    throw new Error(`Unexpected API request: ${url}`);
  });
  await act(async () => root.render(<RunDetail run={{
    id: 'saved-run', jobId: 'storage', name: 'Storage Insights', createdAt: '2026-09-22T09:00:00Z',
    workingDir: '/', scheduleId: 'storage', messageCount: 2, totalTokens: 10, inputTokens: 5, outputTokens: 5,
  }} displayName="Storage Insights" starterId="storage_insights" lastStatus="error" />));
  const button = [...container.querySelectorAll('button')].find(b => b.textContent === 'Recover saved scan results');
  await act(async () => { button!.click(); });
  expect(container.textContent).toContain('Report incomplete — cleanup findings could not be saved.');
  expect(container.textContent).toContain('Saved scan results could not be recovered. incomplete scan findings JSON');
  expect(container.textContent).toContain('Previous valid report — ' + previousDate);
  expect(container.textContent).toContain('Previous storage report');
  expect(container.textContent).toContain('/fixture/previous-cache');
  expect(container.textContent).not.toContain('/fixture/Library/Caches/App');
  expect([...container.querySelectorAll('button')].some(b => /Trash|Review cleanup/.test(b.textContent || ''))).toBe(false);
});

it('does not offer recovery when a ledger is already valid or unverified', async () => {
  for (const [index, ledger] of [
    { ledger_state: 'unverified', findings: [] },
    { ledger_state: 'valid', findings: [recoveredFinding], created_at: '2026-09-10T00:00:00Z' },
    { ledger_state: 'valid', findings: [{ ...recoveredFinding, action_taken: 'trashed', size_recovered_bytes: 100 }], created_at: '2026-09-10T00:00:00Z' },
  ].entries()) {
    vi.mocked(apiFetch).mockImplementation(async path => {
      const url = String(path);
      if (url === '/automation/run/saved-run/findings') return ledger;
      if (url === '/api/sessions/saved-run') return { conversation: [] };
      if (url === '/automation/recovery/total') return { total_recovered_bytes: 0, runs_with_recovery: 0, items_trashed: 0 };
      if (url.startsWith('/schedule/')) return [];
      throw new Error(`Unexpected API request: ${url}`);
    });
    await act(async () => root.render(<RunDetail key={index} run={{
      id: 'saved-run', jobId: 'storage', name: 'Storage Insights', createdAt: '2026-09-22T09:00:00Z',
      workingDir: '/', scheduleId: 'storage', messageCount: 2, totalTokens: 10, inputTokens: 5, outputTokens: 5,
    }} displayName="Storage Insights" starterId="storage_insights" lastStatus="ok" />));
    expect(container.textContent).not.toContain('Recover saved scan results');
    expect(vi.mocked(apiFetch).mock.calls.some(([url]) => String(url).endsWith('/recover-findings'))).toBe(false);
    if (index === 2) expect(container.textContent).toContain('100 B moved to Trash (estimate)');
  }
});


it('missing ledgers offer recovery only for storage cleanup, including legacy job identity', async () => {
  vi.mocked(apiFetch).mockImplementation(async path => {
    const url = String(path);
    if (url.includes('/model-review/')) return [];
    if (url.endsWith('/findings')) return { ledger_state: 'missing', findings: [] };
    if (url.startsWith('/api/sessions/')) return { conversation: [] };
    if (url === '/automation/recovery/total') return { total_recovered_bytes: 0, runs_with_recovery: 0, items_trashed: 0 };
    if (url.startsWith('/schedule/')) return [];
    throw new Error(`Unexpected API request: ${url}`);
  });
  for (const [jobId, starterId, expected] of [
    ['renamed-storage-job', 'storage_insights', true],
    ['storage-insights', undefined, true],
    ['digest', undefined, false],
    ['storage-insights', 'other_starter', false],
    ['model', 'model_review', false],
  ] as const) {
    await act(async () => root.render(<RunDetail key={jobId + starterId} run={{
      id: 'saved-run', jobId, name: 'Fixture', createdAt: '2026-09-22T09:00:00Z',
      workingDir: '/', scheduleId: 'storage-insights', messageCount: 0, totalTokens: 0, inputTokens: 0, outputTokens: 0,
    }} displayName="Fixture" starterId={starterId} />));
    expect(container.textContent?.includes('Recover saved scan results')).toBe(expected);
  }
});

it('switching runs resets recovery state and ignores an old recovery completion or failure', async () => {
  for (const shouldReject of [false, true]) {
    let finishPost: () => void = () => {};
    let rejectPost: (error: Error) => void = () => {};
    const pendingPost = new Promise<void>((resolve, reject) => { finishPost = resolve; rejectPost = reject; });
    const reads: Record<string, number> = {};
    vi.mocked(apiFetch).mockImplementation(async path => {
      const url = String(path);
      if (url.endsWith('/recover-findings')) { await pendingPost; return {}; }
      if (url.endsWith('/findings')) { reads[url] = (reads[url] || 0) + 1; return { ledger_state: 'missing', findings: [] }; }
      if (url.startsWith('/api/sessions/')) return { conversation: [] };
      if (url === '/automation/recovery/total') return { total_recovered_bytes: 0, runs_with_recovery: 0, items_trashed: 0 };
      if (url.startsWith('/schedule/')) return [];
      throw new Error(`Unexpected API request: ${url}`);
    });
    const run = { id: 'old', jobId: 'storage-insights', name: 'Fixture', createdAt: '2026-09-22T09:00:00Z',
      workingDir: '/', scheduleId: 'storage-insights', messageCount: 0, totalTokens: 0, inputTokens: 0, outputTokens: 0 };
    // Same mounted component: changing props must invalidate the old request.
    await act(async () => root.render(<RunDetail key={String(shouldReject)} run={run} displayName="Fixture" starterId="storage_insights" />));
    await act(async () => [...container.querySelectorAll('button')].find(b => b.textContent === 'Recover saved scan results')!.click());
    expect([...container.querySelectorAll('button')].find(b => b.textContent === 'Recovering saved scan results…')?.disabled).toBe(true);
    await act(async () => root.render(<RunDetail key={String(shouldReject)} run={{ ...run, id: 'new' }} displayName="Fixture" starterId="storage_insights" />));
    const newButton = [...container.querySelectorAll('button')].find(b => b.textContent === 'Recover saved scan results');
    expect(newButton?.disabled).toBe(false);
    const readsBefore = reads['/automation/run/new/findings'];
    await act(async () => { if (shouldReject) rejectPost(new Error('OLD recovery error')); else finishPost(); });
    expect(container.textContent).not.toContain('OLD recovery error');
    expect(container.textContent).not.toContain('Recovering saved scan results…');
    expect(reads['/automation/run/new/findings']).toBe(readsBefore);
  }
});


it('a displayed recovery error clears when another run is selected', async () => {
  vi.mocked(apiFetch).mockImplementation(async path => {
    const url = String(path);
    if (url.endsWith('/recover-findings')) throw new Error('OLD run has no saved payload');
    if (url.endsWith('/findings')) return { ledger_state: 'missing', findings: [] };
    if (url.startsWith('/api/sessions/')) return { conversation: [] };
    if (url === '/automation/recovery/total') return { total_recovered_bytes: 0, runs_with_recovery: 0, items_trashed: 0 };
    if (url.startsWith('/schedule/')) return [];
    throw new Error(`Unexpected API request: ${url}`);
  });
  const run = { id: 'old', jobId: 'storage-insights', name: 'Fixture', createdAt: '2026-09-22T09:00:00Z',
    workingDir: '/', scheduleId: 'storage-insights', messageCount: 0, totalTokens: 0, inputTokens: 0, outputTokens: 0 };
  await act(async () => root.render(<RunDetail run={run} displayName="Fixture" starterId="storage_insights" />));
  await act(async () => [...container.querySelectorAll('button')].find(b => b.textContent === 'Recover saved scan results')!.click());
  expect(container.textContent).toContain('OLD run has no saved payload');
  await act(async () => root.render(<RunDetail run={{ ...run, id: 'new' }} displayName="Fixture" starterId="storage_insights" />));
  expect(container.textContent).not.toContain('OLD run has no saved payload');
  expect([...container.querySelectorAll('button')].find(b => b.textContent === 'Recover saved scan results')?.disabled).toBe(false);
});

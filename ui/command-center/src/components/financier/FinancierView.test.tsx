/**
 * @vitest-environment jsdom
 *
 * The Financier tab, pinned against the ways a money surface can lie.
 *
 * Every case here is a claim the tab makes that would be worse than useless if
 * it were wrong: where the data goes, whether a number is real, and whether a
 * control that looks live is actually wired. `../../lib/api` is mocked (the
 * SpendPanel/DataPanel.consent pattern) so mounting touches no network.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: {
    getFinancierRouting: vi.fn(),
    getQuote: vi.fn(),
    getEgressLog: vi.fn(),
    setExtensionEnabled: vi.fn(),
    upsertConfig: vi.fn(),
    getSpend: vi.fn(),
    setBudget: vi.fn(),
  },
  apiFetch: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

import { FinancierView } from './FinancierView';
import { api } from '../../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const routingMock = vi.mocked(api.getFinancierRouting);
const quoteMock = vi.mocked(api.getQuote);
const egressMock = vi.mocked(api.getEgressLog);
const enableMock = vi.mocked(api.setExtensionEnabled);
const upsertMock = vi.mocked(api.upsertConfig);
const spendMock = vi.mocked(api.getSpend);

const LOCAL_ROUTING = {
  kind: 'on_device' as const,
  provider: null,
  model: null,
  is_local: true,
  statement: 'Would run on this Mac, on-device.',
  cloud_allowed: false,
  // Deliberately not the literal the component might otherwise hard-code: the
  // point of this field is that the client uses whatever the daemon names.
  cloud_consent_key: 'financier_allow_cloud',
  enabled: true,
};

const SPEND = {
  runningTotalUsd: 1.5,
  totalTokens: 900,
  sessionCount: 1,
  budget: { session: { soft: 10, gate: 25, hard: 50 }, task: { soft: 2, gate: 5, hard: 10 } },
  sessions: [],
  projects: [],
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  routingMock.mockReset().mockResolvedValue({ ...LOCAL_ROUTING } as never);
  quoteMock.mockReset();
  egressMock.mockReset().mockResolvedValue([] as never);
  enableMock.mockReset().mockResolvedValue('Enabled extension finance' as never);
  upsertMock.mockReset().mockResolvedValue({} as never);
  spendMock.mockReset().mockResolvedValue(SPEND as never);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount() {
  await act(async () => { root.render(<FinancierView />); });
}

function buttonWith(text: string): HTMLButtonElement {
  return Array.from(container.querySelectorAll('button')).find(
    b => b.textContent?.includes(text),
  ) as HTMLButtonElement;
}

/**
 * Type into a React-controlled input.
 *
 * Assigning `.value` directly does not work: React installs its own value
 * setter on the element and reads the previous value from it to decide whether
 * anything changed, so a plain assignment is swallowed and `onChange` never
 * fires. Going through the prototype's setter is the documented way round it.
 */
async function typeSymbol(value: string) {
  const input = container.querySelector('input[type="checkbox"]')
    ? (Array.from(container.querySelectorAll('input')).find(
        el => el.getAttribute('type') !== 'checkbox',
      ) as HTMLInputElement)
    : (container.querySelector('input') as HTMLInputElement);
  const setter = Object.getOwnPropertyDescriptor(
    window.HTMLInputElement.prototype,
    'value',
  )?.set;
  await act(async () => {
    setter?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

describe('The Financier tab', () => {
  it('shows spend on the tab itself — money has one home, not a link to one', async () => {
    await mount();
    // The panel MOVED here; it is rendered, not pointed at. If this fails the
    // consolidation regressed into a second signpost.
    expect(spendMock).toHaveBeenCalled();
    expect(container.textContent).toContain('What your AI work costs');
  });

  it('reports the routing decision the daemon gave, verbatim', async () => {
    await mount();
    expect(container.textContent).toContain('Would run on this Mac, on-device.');
    // The readout must never be mistaken for a record of where a call went —
    // nothing dispatches on this decision yet.
    expect(container.textContent).toContain('not a report of where a call went');
    expect(container.textContent).toContain('on this machine');
  });

  it('never labels a cloud route as local', async () => {
    routingMock.mockResolvedValue({
      ...LOCAL_ROUTING,
      kind: 'cloud',
      provider: 'some-provider',
      model: 'some-model',
      is_local: false,
      cloud_allowed: true,
      statement: 'Would run on a cloud model.',
    } as never);
    await mount();
    expect(container.textContent).toContain('cloud');
    expect(container.textContent).not.toContain('on this machine');
  });

  it('refuses to claim locality when the routing read failed', async () => {
    // The dangerous failure is a surface that falls back to "local" when it
    // does not know. It must say it does not know.
    routingMock.mockRejectedValue(new Error('daemon unreachable'));
    await mount();
    expect(container.textContent).toContain('Could not read the routing decision');
    expect(container.textContent).toContain('daemon unreachable');
    expect(container.textContent).not.toContain('on this machine');
  });

  it('writes the consent key the daemon named, not one it assumed', async () => {
    routingMock.mockResolvedValue({
      ...LOCAL_ROUTING,
      cloud_consent_key: 'a_renamed_consent_key',
    } as never);
    await mount();
    const box = container.querySelector('input[type="checkbox"]') as HTMLInputElement;
    expect(box.checked).toBe(false);
    await act(async () => { box.click(); });
    expect(upsertMock).toHaveBeenCalledWith('a_renamed_consent_key', true);
  });

  it('shows a quote failure verbatim instead of an empty row', async () => {
    quoteMock.mockRejectedValue(new Error('[sovereign] the request was refused at the data boundary'));
    await mount();
    await typeSymbol('TEST');
    await act(async () => { buttonWith('Read').click(); });
    expect(container.textContent).toContain('refused at the data boundary');
  });

  it('renders a price the source did not report as a dash, never as zero', async () => {
    // A `price: 0` rendering as "$0.00" is a wrong number about money, which is
    // the one thing this tab must never do.
    quoteMock.mockResolvedValue({
      symbol: 'TEST',
      price: null,
      previous_close: null,
      change: null,
      change_percent: null,
      day_high: null,
      day_low: null,
      fifty_two_week_high: null,
      fifty_two_week_low: null,
      volume: null,
      quoted_at: null,
      market_closed: false,
    } as never);
    await mount();
    await typeSymbol('TEST');
    await act(async () => { buttonWith('Read').click(); });
    expect(container.textContent).toContain('—');
    expect(container.textContent).not.toContain('0.00');
    expect(container.textContent).toContain('The source gave no timestamp');
  });

  it('offers a working switch when the Financier is off, writing the extension key', async () => {
    // Before this tab existed the capability could be SEEN but not enabled from
    // anywhere in the app — the Tools pane lists extensions read-only.
    routingMock.mockResolvedValue({ ...LOCAL_ROUTING, enabled: false } as never);
    await mount();
    expect(container.textContent).toContain('The Financier is switched off');
    await act(async () => { buttonWith('Enable the Financier').click(); });
    expect(enableMock).toHaveBeenCalledWith('finance', true);
  });

  it('says plainly that it holds no portfolio, rather than showing an empty one', async () => {
    await mount();
    expect(container.textContent).toContain('Holdings, accounts and statements');
    expect(container.textContent).toContain('Permagent stores none of these');
  });

  it('does not overclaim what a quiet egress log proves', async () => {
    await mount();
    expect(container.textContent).toContain('No market data has been fetched');
    // The audit covers inference and market reads, not everything. Saying
    // "nothing left this machine" would be the overclaim.
    expect(container.textContent).toContain('not proof that nothing at all left');
  });

  it('lists only market-data rows in the market-read audit', async () => {
    egressMock.mockResolvedValue([
      { id: '1', ts: new Date().toISOString(), provider: 'https://example.invalid/chart', model: 'TEST', kind: 'market_data', blocked: false, contentHash: 'h', sessionId: null, projectId: null, prompt: null },
      { id: '2', ts: new Date().toISOString(), provider: 'some-provider', model: 'some-model', kind: 'inference', blocked: false, contentHash: 'h', sessionId: null, projectId: null, prompt: null },
    ] as never);
    await mount();
    expect(container.textContent).toContain('TEST');
    // An inference row belongs in the Sovereignty log, not in this narrowed one.
    expect(container.textContent).not.toContain('some-model');
  });
});

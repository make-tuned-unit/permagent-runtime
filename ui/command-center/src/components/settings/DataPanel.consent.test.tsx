/**
 * @vitest-environment jsdom
 *
 * Product-analytics consent wiring (#845, narrowed by the 2026-08
 * finish-the-settings ruling).
 *
 * The panel now carries exactly ONE consent toggle — "Share product
 * analytics" — because the others were fake: "Share anonymous diagnostics"
 * persisted crash_reports_consent that nothing reads, and "Share prompts to
 * improve models" flipped this SAME analytics gate with no prompt-sharing
 * pipeline behind it. These tests pin the surviving contract:
 *   1. the toggle renders from the backend value on load (reflects, never a
 *      hardcoded ON),
 *   2. before the backend answers it is OFF (can't flash consent it doesn't have),
 *   3. flipping it round-trips to the backend consent gate (real, not dead),
 *   4. the removed toggles stay removed.
 *
 * `../../lib/api` is mocked (the GrowView.consume test pattern) so mounting
 * touches no network; the toggle's on/off is observed through the knob transform
 * the Toggle atom renders, and persistence through the api.setCrashConsent call.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: {
    getCrashConsent: vi.fn(),
    setCrashConsent: vi.fn(),
    setAnalyticsConsent: vi.fn(),
    exportCrashReport: vi.fn(),
  },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

// Imported AFTER the mock is registered (vi.mock is hoisted above imports).
import { DataPanel } from './SettingsView';
import { api } from '../../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const getConsentMock = vi.mocked(api.getCrashConsent);
const setConsentMock = vi.mocked(api.setCrashConsent);
const setAnalyticsMock = vi.mocked(api.setAnalyticsConsent);
const exportMock = vi.mocked(api.exportCrashReport);

let container: HTMLDivElement;
let root: Root;

/** The Toggle <button> in the row whose label div has exactly `text`. */
function toggleByLabel(text: string): HTMLButtonElement {
  const label = Array.from(container.querySelectorAll('div')).find(
    d => d.textContent === text,
  );
  if (!label) throw new Error(`row label not found: ${text}`);
  // label div → 200px container → Row → the toggle button in the second column.
  const row = label.parentElement!.parentElement!;
  const btn = row.querySelector('button');
  if (!btn) throw new Error(`toggle button not found for: ${text}`);
  return btn as HTMLButtonElement;
}

/** The product-analytics Toggle. */
function analyticsToggle(): HTMLButtonElement {
  return toggleByLabel('Share product analytics');
}

/** The "Export redacted crash report" button (found by its label text). */
function exportButton(): HTMLButtonElement {
  const btn = Array.from(container.querySelectorAll('button')).find(
    b => (b.textContent || '').includes('Export redacted crash report'),
  );
  if (!btn) throw new Error('export button not found');
  return btn as HTMLButtonElement;
}

/** Toggle atom moves its knob to translateX(14px) when on, translateX(0) when off. */
function isOn(btn: HTMLButtonElement): boolean {
  const knob = btn.querySelector('div');
  return (knob?.getAttribute('style') || '').includes('translateX(14px)');
}

async function mount() {
  await act(async () => {
    root.render(<DataPanel />);
  });
  // Flush the getCrashConsent promise + its state update.
  await act(async () => { await Promise.resolve(); });
}

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  getConsentMock.mockReset();
  setConsentMock.mockReset();
  setAnalyticsMock.mockReset();
  exportMock.mockReset();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('DataPanel analytics consent wiring', () => {
  it('renders the toggle from the backend value (ON when consented)', async () => {
    getConsentMock.mockResolvedValue({ crashReportsConsented: false, analyticsConsented: true });
    await mount();
    expect(getConsentMock).toHaveBeenCalledTimes(1);
    expect(isOn(analyticsToggle())).toBe(true);
  });

  it('renders OFF when the backend reports no consent (default)', async () => {
    getConsentMock.mockResolvedValue({ crashReportsConsented: false, analyticsConsented: false });
    await mount();
    expect(isOn(analyticsToggle())).toBe(false);
  });

  it('is OFF before the backend responds — never a hardcoded ON', async () => {
    // Never-resolving fetch: the toggle must not assume consent while loading.
    getConsentMock.mockReturnValue(new Promise(() => {}));
    await act(async () => { root.render(<DataPanel />); });
    expect(isOn(analyticsToggle())).toBe(false);
  });

  it('persists analytics consent via setAnalyticsConsent (never the crash gate)', async () => {
    getConsentMock.mockResolvedValue({ crashReportsConsented: false, analyticsConsented: false });
    setAnalyticsMock.mockResolvedValue({ crashReportsConsented: false, analyticsConsented: true });
    await mount();

    await act(async () => { analyticsToggle().click(); });
    expect(setAnalyticsMock).toHaveBeenCalledTimes(1);
    expect(setAnalyticsMock).toHaveBeenCalledWith(true);
    expect(setConsentMock).not.toHaveBeenCalled();
    await act(async () => { await Promise.resolve(); });
    expect(isOn(analyticsToggle())).toBe(true);
  });

  it('rolls back the toggle if the persist call fails', async () => {
    getConsentMock.mockResolvedValue({ crashReportsConsented: false, analyticsConsented: false });
    setAnalyticsMock.mockRejectedValue(new Error('boom'));
    await mount();

    await act(async () => { analyticsToggle().click(); });
    await act(async () => { await Promise.resolve(); });
    // Failed write must not leave the UI claiming consent we never stored.
    expect(isOn(analyticsToggle())).toBe(false);
  });

  it('keeps the removed fake toggles removed', async () => {
    getConsentMock.mockResolvedValue({ crashReportsConsented: false, analyticsConsented: false });
    await mount();
    expect(container.textContent).not.toContain('Share anonymous diagnostics');
    expect(container.textContent).not.toContain('Share prompts to improve models');
    expect(setConsentMock).not.toHaveBeenCalled();
  });
});

describe('DataPanel redacted crash-report export (#327)', () => {
  it('calls the real export endpoint and shows the saved path + redacted preview', async () => {
    getConsentMock.mockResolvedValue({ crashReportsConsented: false, analyticsConsented: false });
    exportMock.mockResolvedValue({
      path: '/state/crash-exports/crash-report-x.txt',
      reportCount: 2,
      content: 'Permagent redacted crash-report export\n[REDACTED] boom',
    });
    await mount();

    await act(async () => { exportButton().click(); });
    await act(async () => { await Promise.resolve(); });

    expect(exportMock).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain('/state/crash-exports/crash-report-x.txt');
    expect(container.textContent).toContain('[REDACTED]');
    expect(container.textContent).toContain('2 crash report(s) redacted');
  });

  it('surfaces an error when the export fails', async () => {
    getConsentMock.mockResolvedValue({ crashReportsConsented: false, analyticsConsented: false });
    exportMock.mockRejectedValue(new Error('disk full'));
    await mount();

    await act(async () => { exportButton().click(); });
    await act(async () => { await Promise.resolve(); });
    expect(container.textContent).toContain('Export failed');
  });
});

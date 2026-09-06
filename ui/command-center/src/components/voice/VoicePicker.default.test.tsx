/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ onChange: vi.fn() }));

vi.mock('../../styles/useTheme', () => ({
  useTheme: () => ({ colors: { inputBg: '#111', border: '#333', text: '#fff', surfaceHi: '#222', borderHi: '#555', surface: '#181818', cyan: '#0ff', danger: '#f55', textDim: '#777' } }),
}));
vi.mock('../../lib/useVoices', () => ({
  useVoices: () => ({
    voices: [{ id: 'bf_emma', language: 'en', label: 'English — Emma' }],
    ready: true, loading: false, status: null, downloadPercent: 0, downloadError: null, startDownload: vi.fn(),
  }),
  useVoicePreview: () => ({ preview: vi.fn(), playingId: null, error: null }),
}));
vi.mock('../common/Button', () => ({ Button: ({ children, onClick }: { children?: React.ReactNode; onClick?: () => void }) => <button onClick={onClick}>{children}</button> }));

import { VoicePicker } from './VoicePicker';

describe('VoicePicker default persistence', () => {
  let container: HTMLDivElement;
  let root: Root;

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    mocks.onChange.mockReset();
  });

  it('preserves intentional null outside onboarding', () => {
    container = document.createElement('div');
    document.body.append(container);
    root = createRoot(container);
    act(() => root.render(<VoicePicker value={null} onChange={mocks.onChange} />));
    expect(container.querySelector('select')?.value).toBe('bf_emma');
    expect(mocks.onChange).not.toHaveBeenCalled();
  });

  it('persists the same default voice that onboarding displays', () => {
    container = document.createElement('div');
    document.body.append(container);
    root = createRoot(container);
    act(() => root.render(<VoicePicker seedDefault value={null} onChange={mocks.onChange} />));
    expect(container.querySelector('select')?.value).toBe('bf_emma');
    expect(mocks.onChange).toHaveBeenCalledWith('bf_emma');
  });
});

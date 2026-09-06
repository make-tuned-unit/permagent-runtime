// @vitest-environment jsdom
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MomentHardware } from './MomentHardware';
import { MomentCode } from './MomentCode';
import { MomentWebSearch } from './MomentWebSearch';
import { MomentChat } from './MomentChat';

const mocks = vi.hoisted(() => ({
  getSystemInfo: vi.fn(),
  getOllamaStatus: vi.fn(),
  startOllama: vi.fn(),
  pullOllamaModel: vi.fn(),
  getLibrarianSchedule: vi.fn(),
  setLibrarianSchedule: vi.fn(),
  readSecretConfig: vi.fn(),
  probeExtension: vi.fn(),
  getDevRoots: vi.fn(),
  upsertConfig: vi.fn(),
}));

vi.mock('../../lib/api', () => ({ api: mocks }));
vi.mock('../../styles/useTheme', () => ({
  useTheme: () => ({
    reduceMotion: true,
    theme: 'graphite',
    colors: {
      bg: '#0b1020', text: '#fff', textMuted: '#aaa', textDim: '#777',
      cyan: '#0ff', cyanSoft: '#033', cyanGlow: '#055', purple: '#80f',
      purpleGlow: '#204', purpleSoft: '#303', purpleBright: '#a0f',
      border: '#334', borderHi: '#556', inputBg: '#111827', surface: '#172033',
      surfaceHi: '#22304a', success: '#0f8', warning: '#fa0', danger: '#f55',
      bgDeeper: '#070b16', textOnAccent: '#fff', cardShadow: 'none',
    },
  }),
}));
vi.mock('../mobius/Mobius', () => ({ Mobius: () => <div data-testid="mobius" /> }));
vi.mock('./atoms', () => ({
  Glass: ({ children }: { children?: React.ReactNode }) => <div>{children}</div>,
  GhostLink: ({ children, onClick }: { children?: React.ReactNode; onClick?: () => void }) => <button onClick={onClick}>{children}</button>,
  Input: ({ value, onChange }: { value: string; onChange: (value: string) => void }) => (
    <input value={value} onChange={event => onChange(event.target.value)} />
  ),
  Particles: () => null,
  PrimaryButton: ({ children, onClick, disabled }: { children?: React.ReactNode; onClick?: () => void; disabled?: boolean }) => (
    <button onClick={onClick} disabled={disabled}>{children}</button>
  ),
  Select: ({ value, onChange }: { value: string; onChange: (value: string) => void }) => (
    <select value={value} onChange={event => onChange(event.target.value)} />
  ),
  WizardHeading: ({ children }: { children?: React.ReactNode }) => <h1>{children}</h1>,
  WizardSubhead: ({ children }: { children?: React.ReactNode }) => <p>{children}</p>,
}));
vi.mock('../common/Button', () => ({
  Button: ({ children, onClick, disabled }: { children?: React.ReactNode; onClick?: () => void; disabled?: boolean }) => (
    <button onClick={onClick} disabled={disabled}>{children}</button>
  ),
}));
vi.mock('../voice/VoicePicker', () => ({ VoicePicker: () => <div /> }));

let root: Root | null = null;
let container: HTMLDivElement | null = null;

afterEach(() => {
  if (root) act(() => root?.unmount());
  container?.remove();
  root = null;
  container = null;
  vi.clearAllMocks();
});

function mount(element: React.ReactElement) {
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
  act(() => root?.render(element));
}

describe('wizard inactive-step lifecycle', () => {
  it('does not scan hardware or start Ollama until the hardware Moment is active', async () => {
    mocks.getSystemInfo.mockResolvedValue({
      total_ram_bytes: 16 * 1024 * 1024 * 1024,
      disk_free_bytes: 20 * 1024 * 1024 * 1024,
      cpu_brand: 'Fixture CPU', architecture: 'fixture',
    });
    mocks.getOllamaStatus.mockResolvedValue({ reachable: true, installed: [], running: [] });
    mocks.startOllama.mockResolvedValue({ launched: true, method: 'fixture' });

    mount(<MomentHardware active={false} onAdvance={vi.fn()} onBack={vi.fn()} />);
    expect(mocks.getSystemInfo).not.toHaveBeenCalled();
    expect(mocks.startOllama).not.toHaveBeenCalled();

    await act(async () => root?.render(<MomentHardware active onAdvance={vi.fn()} onBack={vi.fn()} />));
    expect(mocks.getSystemInfo).toHaveBeenCalledOnce();
    expect(mocks.startOllama).not.toHaveBeenCalled();
  });

  it('ignores a late Ollama response after the hardware step is deactivated', async () => {
    let resolveStatus: ((value: { reachable: boolean; installed: never[]; running: never[] }) => void) | undefined;
    mocks.getSystemInfo.mockResolvedValue({
      total_ram_bytes: 16 * 1024 * 1024 * 1024,
      disk_free_bytes: 20 * 1024 * 1024 * 1024,
      cpu_brand: 'Fixture CPU', architecture: 'fixture',
    });
    mocks.getOllamaStatus.mockReturnValue(new Promise(resolve => { resolveStatus = resolve; }));

    mount(<MomentHardware active onAdvance={vi.fn()} onBack={vi.fn()} />);
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    act(() => root?.render(<MomentHardware active={false} onAdvance={vi.fn()} onBack={vi.fn()} />));
    await act(async () => {
      resolveStatus?.({ reachable: false, installed: [], running: [] });
      await Promise.resolve();
    });

    expect(container?.textContent).toContain('Scanning your system');
    expect(container?.textContent).not.toContain('Starting the local model runtime');
  });

  it('does not apply a stale installed-model result after leaving the step', async () => {
    let resolveInstalled: ((value: { reachable: boolean; installed: Array<{ name: string }>; running: never[] }) => void) | undefined;
    mocks.getSystemInfo.mockResolvedValue({
      total_ram_bytes: 16 * 1024 * 1024 * 1024,
      disk_free_bytes: 20 * 1024 * 1024 * 1024,
      cpu_brand: 'Fixture CPU', architecture: 'fixture',
    });
    mocks.getOllamaStatus
      .mockResolvedValueOnce({ reachable: true, installed: [], running: [] })
      .mockReturnValueOnce(new Promise(resolve => { resolveInstalled = resolve; }));

    mount(<MomentHardware active onAdvance={vi.fn()} onBack={vi.fn()} />);
    await act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); });
    act(() => root?.render(<MomentHardware active={false} onAdvance={vi.fn()} onBack={vi.fn()} />));
    await act(async () => {
      resolveInstalled?.({ reachable: true, installed: [{ name: 'qwen3:8b' }], running: [] });
      await Promise.resolve();
    });

    expect(Array.from(container?.querySelectorAll('button') ?? []).some(b => b.textContent?.startsWith('Enable the Librarian'))).toBe(false);
  });

  it('does not transition to ready when a model pull resolves after deactivation', async () => {
    let resolvePull: (() => void) | undefined;
    mocks.getSystemInfo.mockResolvedValue({
      total_ram_bytes: 16 * 1024 * 1024 * 1024,
      disk_free_bytes: 20 * 1024 * 1024 * 1024,
      cpu_brand: 'Fixture CPU', architecture: 'fixture',
    });
    mocks.getOllamaStatus.mockResolvedValue({ reachable: true, installed: [], running: [] });
    mocks.getLibrarianSchedule.mockResolvedValue({ enabled: false, start_time: '03:00', duration_minutes: 10, model: 'qwen3:8b', run_if_launched_in_window: false });
    mocks.setLibrarianSchedule.mockResolvedValue({});
    mocks.pullOllamaModel.mockImplementation(() => ({
      promise: new Promise<void>(resolve => { resolvePull = resolve; }),
      abort: vi.fn(),
    }));

    mount(<MomentHardware active onAdvance={vi.fn()} onBack={vi.fn()} />);
    await act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); });
    const install = Array.from(container?.querySelectorAll('button') ?? []).find(b => b.textContent?.startsWith('Install '));
    expect(install).toBeDefined();
    act(() => install?.click());
    act(() => root?.render(<MomentHardware active={false} onAdvance={vi.fn()} onBack={vi.fn()} />));
    await act(async () => {
      resolvePull?.();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(container?.textContent).not.toContain('Ready to go');
  });

  it('does not read search-provider secrets until the Web Search Moment is active', async () => {
    mocks.readSecretConfig.mockResolvedValue(null);
    mount(<MomentWebSearch active={false} onAdvance={vi.fn()} onBack={vi.fn()} />);
    expect(mocks.readSecretConfig).not.toHaveBeenCalled();

    await act(async () => root?.render(<MomentWebSearch active onAdvance={vi.fn()} onBack={vi.fn()} />));
    expect(mocks.readSecretConfig).toHaveBeenCalledTimes(2);
  });

  it('shows a saved search key as unverified until a live probe succeeds', async () => {
    mocks.readSecretConfig.mockResolvedValue({ maskedValue: '••••1234' });
    mount(<MomentWebSearch active onAdvance={vi.fn()} onBack={vi.fn()} />);
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(container?.textContent).toContain('Saved — test to verify');
    expect(container?.textContent).not.toContain('Connected');
    expect(container?.textContent).toContain('Test saved key');
  });

  it('keeps a repository scan failure visible and retries it explicitly', async () => {
    mocks.getDevRoots
      .mockRejectedValueOnce(new Error('daemon unavailable'))
      .mockResolvedValueOnce({ confirmed: [], discovered: [], home: '/Users/test' });
    mount(<MomentCode active onAdvance={vi.fn()} onBack={vi.fn()} />);
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(container?.textContent).toContain("couldn't scan for repositories");

    const retry = Array.from(container?.querySelectorAll('button') ?? []).find(b => b.textContent === 'Retry scan');
    expect(retry).toBeDefined();
    await act(async () => retry?.click());
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(container?.textContent).not.toContain("couldn't scan for repositories");
    expect(mocks.getDevRoots).toHaveBeenCalledTimes(2);
  });

  it('does not start the first-chat greeting while Chat is inactive', () => {
    const persona = { name: 'Aria', traits: ['helpful'], tone: 'warm', greeting: 'A real greeting' };
    mount(<MomentChat active={false} persona={persona} onComplete={vi.fn()} />);
    expect(container?.textContent).not.toContain('A real greeting');

    act(() => root?.render(<MomentChat active persona={persona} onComplete={vi.fn()} />));
    expect(container?.textContent).toContain('A real greeting');
    expect(container?.textContent).toContain('Preview');
    expect(container?.textContent).not.toContain('Online');
  });
});

import { useCallback, useEffect, useId, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { FiChevronDown } from 'react-icons/fi';
import { font, radius, space, textSize, duration, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { api } from '../../lib/api';
import { SEARCH_PROVIDERS, saveAndEnableSearchProvider } from '../../lib/searchProviders';
import { recommendModel, MIN_RAM_GB, type ModelTier } from '../../lib/modelTiers';
import { hasInstalledModel, memoryStatus, memoryStatusLabel, type MemoryStatus } from './memoryStatus';
import { FieldLabel, GhostLink, Input, SecondaryButton, Select, StatusLine } from './atoms';
import { StepFrame } from './StepFrame';
import { safeWizardError } from './wizardErrors';

interface Props {
  agentName: string;
  onNext: () => void;
}

type Section = 'search' | 'memory' | null;

/**
 * Two optional extras, each closed until asked for: web search, and a small
 * local model that tidies memory. Nothing starts, downloads or connects until
 * the person presses the button that says so.
 */
export function StepExtras({ agentName, onNext }: Props) {
  const [open, setOpen] = useState<Section>(null);
  const [searchState, setSearchState] = useState<'none' | 'saved' | 'working'>('none');
  const [memoryState, setMemoryState] = useState<MemoryStatus>('checking');
  const [memoryPreference, setMemoryPreference] = useState<boolean | null>(null);
  const [downloading, setDownloading] = useState(false);
  // A saved enabled schedule remains the person's choice even while its model
  // is unavailable. Never imply that Continue/Skip changed that preference.
  const anything = searchState !== 'none' || memoryPreference !== false;

  return (
    <StepFrame
      title="Extras"
      why="Both optional. Both can be added later in Settings."
      primary={{ label: anything ? 'Continue' : 'Skip for now' }}
      onPrimary={onNext}
      secondary={downloading ? <span style={{ fontFamily: font.body, fontSize: textSize.caption }}>Leaving pauses the download.</span> : undefined}
    >
      <Section
        title="Web search"
        detail={`Lets ${agentName} look things up. Needs a key from a search service; your searches go to that service.`}
        status={searchState === 'working' ? 'Working' : searchState === 'saved' ? 'Key saved' : 'Not set up'}
        good={searchState === 'working'}
        open={open === 'search'}
        onToggle={() => setOpen(o => (o === 'search' ? null : 'search'))}
        autoFocus
      >
        <SearchSetup agentName={agentName} onState={setSearchState} />
      </Section>
      <Section
        title="Local memory model"
        detail="A small model on this Mac that tidies and links your memories overnight. Nothing it reads leaves your Mac."
        status={memoryStatusLabel(memoryState)}
        good={memoryState === 'installed'}
        open={open === 'memory'}
        onToggle={() => setOpen(o => (o === 'memory' ? null : 'memory'))}
      >
        <MemorySetup onState={setMemoryState} onPreference={setMemoryPreference} onDownloading={setDownloading} />
      </Section>
    </StepFrame>
  );
}

function Section({ title, detail, status, good, open, onToggle, autoFocus, children }: {
  title: string; detail: string; status: string; good: boolean; open: boolean;
  onToggle: () => void; autoFocus?: boolean; children: ReactNode;
}) {
  const { colors } = useTheme();
  const bodyId = useId();
  return (
    <div style={{ borderRadius: radius.lg, border: `1px solid ${open ? colors.borderHi : colors.border}`, background: colors.fillSubtle }}>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={bodyId}
        onClick={onToggle}
        {...(autoFocus ? { 'data-autofocus': '' } : {})}
        className="pa-btn"
        style={{
          '--pa-btn-bg': 'transparent', '--pa-btn-bg-hover': colors.fillHover, '--pa-btn-bg-active': colors.fillActive,
          '--pa-btn-fg': colors.text, '--pa-btn-border': 'transparent', '--pa-btn-border-hover': 'transparent',
          '--pa-btn-pad': `${space.xl}px ${space.xxl}px`, '--pa-btn-radius': `${radius.lg}px`, '--pa-btn-weight': 400,
          width: '100%', display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: space.xl, textAlign: 'left',
        } as CSSProperties}
      >
        <span style={{ display: 'flex', flexDirection: 'column', gap: space.xxs }}>
          <span style={{ fontFamily: font.body, fontSize: textSize.body, fontWeight: 600, color: colors.text }}>{title}</span>
          <span style={{ fontFamily: font.body, fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.45 }}>{detail}</span>
        </span>
        <span style={{ display: 'flex', alignItems: 'center', gap: space.md, flexShrink: 0 }}>
          <span style={{ fontFamily: font.body, fontSize: textSize.caption, fontWeight: 600, color: good ? colors.success : colors.textMuted }}>{status}</span>
          <FiChevronDown size={14} aria-hidden style={{ color: colors.textMuted, transform: open ? 'rotate(180deg)' : 'none', transition: `transform ${duration.base}ms ${ease.out}` }} />
        </span>
      </button>
      {/* Kept mounted while closed so a running download or test is not lost
          by collapsing its section. */}
      <div id={bodyId} hidden={!open} style={{ padding: `0 ${space.xxl}px ${space.xxl}px` }}>
        {children}
      </div>
    </div>
  );
}

// ── Web search ─────────────────────────────────────────────────────────

function SearchSetup({ agentName, onState }: { agentName: string; onState: (s: 'none' | 'saved' | 'working') => void }) {
  const { colors } = useTheme();
  const keyId = useId();
  const [providerId, setProviderId] = useState('');
  const [saved, setSaved] = useState<Record<string, boolean>>({});
  const [working, setWorking] = useState<Record<string, boolean>>({});
  const [key, setKey] = useState('');
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ tone: 'ok' | 'warn' | 'error'; text: string } | null>(null);
  const [showSteps, setShowSteps] = useState(false);
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);

  // Read-back: a key saved by an earlier run is shown as saved, not blank.
  useEffect(() => {
    void Promise.all(SEARCH_PROVIDERS.map(async p => {
      try {
        const r = await api.readSecretConfig(p.keyName);
        return [p.id, !!r?.maskedValue] as const;
      } catch {
        return [p.id, false] as const;
      }
    })).then(entries => { if (alive.current) setSaved(Object.fromEntries(entries)); });
  }, []);

  useEffect(() => {
    onState(Object.values(working).some(Boolean) ? 'working' : Object.values(saved).some(Boolean) ? 'saved' : 'none');
  }, [saved, working, onState]);

  const provider = SEARCH_PROVIDERS.find(p => p.id === providerId);

  const probe = async () => {
    if (!provider) return false;
    const r = await api.probeExtension(provider.displayName);
    if (!alive.current) return false;
    if (r.ok) {
      setWorking(w => ({ ...w, [provider.id]: true }));
      setResult({ tone: 'ok', text: `Working. ${agentName} can search the web.` });
    } else {
      setResult({ tone: 'warn', text: `The key is saved, but ${provider.displayName} didn't answer${r.error ? ` (${r.error})` : ''}. Check the whole key was copied; a new key can take a minute to activate.` });
    }
    return r.ok;
  };

  const saveAndTest = async () => {
    if (!provider || !key.trim()) return false;
    setBusy(true);
    setResult(null);
    try {
      await saveAndEnableSearchProvider(provider, key);
      if (!alive.current) return false;
      setSaved(s => ({ ...s, [provider.id]: true }));
      setKey('');
      return await probe();
    } catch (e) {
      if (!alive.current) return false;
      console.error('Search setup failed:', safeWizardError(e, 'search provider setup failed'));
      setResult({ tone: 'error', text: e instanceof Error ? e.message : "Couldn't save the key." });
      return false;
    } finally {
      if (alive.current) setBusy(false);
    }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: space.xl }}>
      <div>
        <FieldLabel htmlFor={`${keyId}-svc`}>Search service</FieldLabel>
        <Select id={`${keyId}-svc`} ariaLabel="Search service" value={providerId} placeholder="Choose a service"
          onChange={v => { setProviderId(v); setKey(''); setResult(null); setShowSteps(false); }}
          options={SEARCH_PROVIDERS.map(p => ({ value: p.id, label: p.displayName, note: working[p.id] ? 'Working' : saved[p.id] ? 'Key saved' : undefined }))} />
      </div>
      {provider && (
        <>
          <div>
            <FieldLabel htmlFor={keyId}>API key</FieldLabel>
            <div style={{ display: 'flex', gap: space.md }}>
              <Input id={keyId} type="password" value={key} onChange={v => { setKey(v); setResult(null); }}
                onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); void saveAndTest(); } }}
                placeholder={saved[provider.id] ? 'Key saved. Paste a new one to replace it.' : `Paste your ${provider.displayName} key`} />
              <SecondaryButton onClick={saveAndTest} disabled={busy || !key.trim()}>{busy ? 'Testing…' : 'Save & test'}</SecondaryButton>
            </div>
          </div>
          {busy && <StatusLine tone="info">Saving and testing…</StatusLine>}
          {result && <StatusLine tone={result.tone}>{result.text}</StatusLine>}
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.xl }}>
            <GhostLink onClick={() => { window.open(provider.keyPageUrl, '_blank', 'noopener,noreferrer'); setShowSteps(true); }}>Open {provider.keyPageLabel} ↗</GhostLink>
            {saved[provider.id] && !working[provider.id] && <GhostLink onClick={probe}>Test saved key</GhostLink>}
          </div>
          {showSteps && (
            <ol style={{ margin: 0, paddingLeft: space.xxxl, display: 'flex', flexDirection: 'column', gap: space.xs }}>
              {provider.setupSteps.map((s, i) => (
                <li key={i} style={{ fontFamily: font.body, fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.5 }}>{s}</li>
              ))}
            </ol>
          )}
        </>
      )}
    </div>
  );
}

// ── Local memory model (the Librarian) ─────────────────────────────────

interface Schedule {
  enabled: boolean; start_time: string; duration_minutes: number; model: string;
  run_if_launched_in_window: boolean; pruning_enabled?: boolean;
}

function MemorySetup({ onState, onPreference, onDownloading }: {
  onState: (s: MemoryStatus) => void;
  onPreference: (enabled: boolean) => void;
  onDownloading: (d: boolean) => void;
}) {
  const { colors } = useTheme();
  const [ramGB, setRamGB] = useState<number | null>(null);
  const [tier, setTier] = useState<ModelTier | null>(null);
  const [reachable, setReachable] = useState<boolean | null>(null);
  const [statusUnavailable, setStatusUnavailable] = useState(false);
  const [installed, setInstalled] = useState<string[]>([]);
  const [schedule, setSchedule] = useState<Schedule | null>(null);
  const [scheduleUnavailable, setScheduleUnavailable] = useState(false);
  const [starting, setStarting] = useState(false);
  const [pct, setPct] = useState<number | null>(null);
  const [note, setNote] = useState<{ tone: 'ok' | 'warn' | 'error'; text: string } | null>(null);
  const abortRef = useRef<(() => void) | null>(null);
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; abortRef.current?.(); }; }, []);

  const readStatus = useCallback(async () => {
    try {
      const s = await api.getOllamaStatus();
      if (!alive.current) return null;
      setStatusUnavailable(false);
      setReachable(s.reachable);
      setInstalled(s.installed.map(m => m.name));
      return s;
    } catch {
      if (alive.current) {
        setStatusUnavailable(true);
        setReachable(null);
        setInstalled([]);
      }
      return null;
    }
  }, []);

  // Read-only look at the machine: memory size, Ollama status, current schedule.
  useEffect(() => {
    api.getSystemInfo()
      .then(info => {
        if (!alive.current) return;
        const bytes = info.total_ram_bytes || 0;
        setRamGB(Math.round(bytes / 1024 ** 3));
        setTier(recommendModel(bytes));
      })
      .catch(() => { if (alive.current) setRamGB(0); });
    void readStatus();
    api.getLibrarianSchedule()
      .then(s => { if (alive.current) { setSchedule(s); setScheduleUnavailable(false); } })
      .catch(() => { if (alive.current) setScheduleUnavailable(true); });
  }, [readStatus]);

  const status = memoryStatus({
    schedule, scheduleUnavailable, ollamaUnavailable: statusUnavailable,
    ollama: reachable === null ? null : { reachable, installed: installed.map(name => ({ name })) },
  });
  useEffect(() => { onState(status); }, [status, onState]);
  useEffect(() => { if (schedule) onPreference(schedule.enabled); }, [schedule, onPreference]);
  useEffect(() => { onDownloading(pct !== null); }, [pct, onDownloading]);

  const model = tier?.model ?? '';
  const hasModel = hasInstalledModel(installed, model);

  const writeSchedule = async (enabled: boolean) => {
    const base = schedule ?? await api.getLibrarianSchedule();
    const next = { ...base, enabled, model: enabled ? model : base.model };
    await api.setLibrarianSchedule(next);
    if (alive.current) setSchedule(next);
  };

  const start = async () => {
    setStarting(true);
    try { await api.startOllama(); } catch { /* polled below */ }
    for (let i = 0; i < 8 && alive.current; i += 1) {
      await new Promise(r => setTimeout(r, 2000));
      if ((await readStatus())?.reachable) break;
    }
    if (alive.current) setStarting(false);
  };

  const turnOn = async () => {
    setNote(null);
    try {
      if (!hasModel) {
        setPct(0);
        const job = api.pullOllamaModel(model, d => {
          if (alive.current && d.total && d.completed) setPct(Math.round((d.completed / d.total) * 100));
        });
        abortRef.current = job.abort;
        await job.promise;
        abortRef.current = null;
        if (!alive.current) return false;
        setPct(null);
      }
      const latest = await readStatus();
      if (!latest?.reachable || !hasInstalledModel(latest.installed.map(item => item.name), model)) {
        if (alive.current) setNote({ tone: 'warn', text: `${model} is not listed in Ollama yet. Check again before turning the schedule on.` });
        return false;
      }
      await writeSchedule(true);
      // The badge reports both the saved choice and installed-model evidence.
      if (alive.current) setNote(null);
      return true;
    } catch (e) {
      abortRef.current = null;
      if (!alive.current) return false;
      setPct(null);
      setNote((e as Error).name === 'AbortError'
        ? { tone: 'warn', text: 'Download stopped. Starting again picks up where it left off.' }
        : { tone: 'error', text: `Couldn't turn it on: ${e instanceof Error ? e.message : String(e)}` });
      return false;
    }
  };

  const turnOff = async () => {
    try {
      await writeSchedule(false);
      setNote(null);
      return true;
    } catch (e) {
      setNote({ tone: 'error', text: `Couldn't turn it off: ${e instanceof Error ? e.message : String(e)}` });
      return false;
    }
  };

  const text = { fontFamily: font.body, fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.5, margin: 0 } as const;

  if (ramGB === null || (reachable === null && !statusUnavailable)) return <StatusLine tone="info">Checking this Mac…</StatusLine>;
  if (!tier) {
    return <p style={text}>This Mac has {ramGB} GB of memory, which is under the {MIN_RAM_GB} GB a local model needs. Everything else works without it.</p>;
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: space.lg }}>
      <p style={text}>
        Suggested for {ramGB} GB of memory: <span style={{ fontFamily: font.mono, color: colors.text }}>{model}</span>
        {hasModel ? ', already installed.' : `, about ${tier.downloadSizeGB} GB to download.`} Runs with Ollama.
      </p>

      {statusUnavailable && <StatusLine tone="warn">Could not check Ollama. The saved memory schedule has not been changed. <GhostLink onClick={readStatus}>Check again</GhostLink>{schedule?.enabled && <> · <GhostLink onClick={turnOff}>Turn off schedule</GhostLink></>}</StatusLine>}
      {scheduleUnavailable && <StatusLine tone="warn">Could not read the memory schedule. Check again later before changing it.</StatusLine>}
      {schedule?.enabled && status === 'needs-ollama' && <StatusLine tone="warn">The schedule is on for {schedule.model}, but Ollama is not reachable, so the model cannot run yet.</StatusLine>}
      {schedule?.enabled && status === 'needs-model' && <StatusLine tone="warn">The schedule is on for {schedule.model}, but Ollama does not list that model.</StatusLine>}

      {!statusUnavailable && !reachable && (
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.md, alignItems: 'center' }}>
          <SecondaryButton onClick={start} disabled={starting}>{starting ? 'Starting…' : 'Start Ollama'}</SecondaryButton>
          <SecondaryButton onClick={() => { window.open('https://ollama.com/download', '_blank', 'noopener,noreferrer'); }}>Get Ollama ↗</SecondaryButton>
          <GhostLink onClick={readStatus}>Check again</GhostLink>
          {schedule?.enabled && <GhostLink onClick={turnOff}>Turn off schedule</GhostLink>}
        </div>
      )}

      {reachable && pct === null && (
        status === 'installed'
          ? <div style={{ display: 'flex', gap: space.xl, alignItems: 'center' }}>
              <StatusLine tone="ok">On · {schedule?.model} is installed in Ollama</StatusLine>
              <GhostLink onClick={turnOff}>Turn off</GhostLink>
            </div>
          : <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.md, alignItems: 'center' }}>
              <SecondaryButton onClick={turnOn} disabled={scheduleUnavailable}>{schedule?.enabled
                ? hasModel ? `Use installed ${model} instead` : `Download ${tier.downloadSizeGB} GB and use ${model} instead`
                : hasModel ? 'Turn on' : `Download ${tier.downloadSizeGB} GB and turn on`}</SecondaryButton>
              {schedule?.enabled && <GhostLink onClick={turnOff}>Turn off</GhostLink>}
            </div>
      )}

      {pct !== null && (
        <div>
          <div role="progressbar" aria-label={`Downloading ${model}`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}
            style={{ height: 6, borderRadius: radius.pill, background: colors.fillActive, overflow: 'hidden' }}>
            <div style={{ width: `${pct}%`, height: '100%', background: colors.cyan, transition: `width ${duration.slow}ms ${ease.out}` }} />
          </div>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginTop: space.sm }}>
            <span style={{ fontFamily: font.mono, fontSize: textSize.caption, color: colors.textMuted }}>Downloading {model} · {pct}%</span>
            <SecondaryButton onClick={() => abortRef.current?.()}>Cancel</SecondaryButton>
          </div>
        </div>
      )}

      {note && <StatusLine tone={note.tone}>{note.text}</StatusLine>}
    </div>
  );
}

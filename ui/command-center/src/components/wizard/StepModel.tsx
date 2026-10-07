import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { font, radius, space, textSize, duration, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { api, type ProviderCheckResult, type SecretBackendStatus } from '../../lib/api';
import { recommendModel } from '../../lib/modelTiers';
import { REFERENCE_HINTS, buildSpec, suggestedManager, type SourceKind } from '../settings/secretSource';
import { ChoiceCard, FieldLabel, GhostLink, Input, Panel, SecondaryButton, Select, StatusLine } from './atoms';
import { StepFrame, type StepPrimary } from './StepFrame';
import { keyProviders, hasLocalRuntime, splitStepLink, LOCAL_RUNTIME_PROVIDER, type KeyProvider, type ProviderEntry } from './providerChoices';
import type { ModelChoice } from './wizardDraft';
import { safeWizardError } from './wizardErrors';

interface Props {
  initialChoice: ModelChoice | null;
  onDone: (choice: ModelChoice) => void;
}

/** A panel's contribution to the step footer. */
interface PanelAction {
  primary: StepPrimary;
  run: () => unknown;
}

/** Strip the route's "Provider 'x' check failed: " prefix — the person already
 *  knows which provider they picked. */
export function friendlyCheckError(message: string): string {
  const text = message.replace(/^Provider '[^']*' check (?:failed|task failed):\s*/, '').trim() || message;
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** What a finished check says, as one line. */
export function describeCheck(result: ProviderCheckResult | undefined): { tone: 'ok' | 'warn'; text: string } {
  if (!result) {
    return { tone: 'warn', text: "Saved. This version of the background service can't confirm keys live, so it will be tried on your first message." };
  }
  if (result.live) return { tone: 'ok', text: `Key works · ${result.model}` };
  return { tone: 'warn', text: `Saved, but it couldn't be confirmed live${result.note ? `: ${result.note}` : '.'} You can continue and it will be tried on your first message.` };
}

/**
 * How the agent thinks: a hosted provider with an API key, a model on this
 * Mac, or "decide later". Nothing is pre-selected; the provider list comes
 * from the daemon's registry; a key is only "working" after a live call.
 */
export function StepModel({ initialChoice, onDone }: Props) {
  const [providers, setProviders] = useState<ProviderEntry[] | null>(null);
  const [loadError, setLoadError] = useState('');
  const [choice, setChoice] = useState<ModelChoice | null>(initialChoice);
  const [action, setAction] = useState<PanelAction | null>(null);

  const load = useCallback(() => {
    setLoadError('');
    api.getProviders()
      .then(list => {
        setProviders(list);
        // Resume: if a provider is already the default, the person chose it
        // earlier — reflect that choice instead of a blank form.
        setChoice(c => {
          if (c) return c;
          const def = list.find(p => p.is_configured && p.is_default);
          if (!def) return null;
          return def.name === LOCAL_RUNTIME_PROVIDER ? 'local' : 'hosted';
        });
      })
      .catch(e => setLoadError(e instanceof Error ? e.message : String(e)));
  }, []);
  useEffect(() => { load(); }, [load]);

  const hosted = providers ? keyProviders(providers) : [];
  const localAvailable = providers ? hasLocalRuntime(providers) : false;

  const primary: StepPrimary = choice === 'later'
    ? { label: 'Continue without a model' }
    : action?.primary ?? { label: 'Continue', disabled: true };
  const onPrimary = () => {
    if (choice === 'later') { onDone('later'); return; }
    return action?.run();
  };

  return (
    <StepFrame
      title="Choose a model"
      why="Permagent needs a language model to think with. Use a hosted provider with an API key, or run one on this Mac."
      primary={primary}
      onPrimary={onPrimary}
      width={680}
    >
      <div role="radiogroup" aria-label="How your agent thinks" style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(190px, 1fr))', gap: space.md }}>
        <ChoiceCard name="model-choice" value="hosted" checked={choice === 'hosted'} onSelect={() => setChoice('hosted')}
          title="Hosted provider" detail="Best quality. Needs an API key from the provider." autoFocus />
        {localAvailable && (
          <ChoiceCard name="model-choice" value="local" checked={choice === 'local'} onSelect={() => setChoice('local')}
            title="On this Mac" detail="Private, and free to run. Needs Ollama and a few GB of disk." />
        )}
        <ChoiceCard name="model-choice" value="later" checked={choice === 'later'} onSelect={() => setChoice('later')}
          title="Decide later" detail="Look around first. Your agent can't reply until a model is connected." />
      </div>

      {loadError && (
        <StatusLine tone="error">
          Couldn't load the provider list ({loadError}).{' '}
          <GhostLink onClick={load}>Try again</GhostLink>
        </StatusLine>
      )}

      {choice === 'hosted' && providers && (
        <HostedPanel providers={hosted} onAction={setAction} onDone={() => onDone('hosted')} />
      )}
      {choice === 'local' && providers && (
        <LocalPanel onAction={setAction} onDone={() => onDone('local')} />
      )}
      {choice === 'later' && (
        <StatusLine tone="info">You can connect a model any time in Settings → Models.</StatusLine>
      )}
    </StepFrame>
  );
}

// ── Hosted ─────────────────────────────────────────────────────────────

type HostedPhase = 'idle' | 'testing' | 'ok' | 'inconclusive' | 'error';

function HostedPanel({ providers, onAction, onDone }: {
  providers: KeyProvider[];
  onAction: (a: PanelAction | null) => void;
  onDone: () => void;
}) {
  const { colors } = useTheme();
  const keyId = useId();
  const noteId = useId();
  const initial = providers.find(p => p.isConfigured && p.isDefault)?.name ?? '';
  const [provider, setProvider] = useState(initial);
  const [key, setKey] = useState('');
  const [phase, setPhase] = useState<HostedPhase>('idle');
  const [message, setMessage] = useState('');
  const [showHelp, setShowHelp] = useState(false);
  const [manager, setManager] = useState<SecretBackendStatus | null>(null);
  const [useReference, setUseReference] = useState(false);
  const [reference, setReference] = useState('');
  /** Providers whose key was saved during this visit (the list is a snapshot). */
  const [savedHere, setSavedHere] = useState<Set<string>>(new Set());
  const generation = useRef(0);
  useEffect(() => () => { generation.current += 1; }, []);

  useEffect(() => {
    let alive = true;
    api.getSecretSources()
      .then(r => { if (alive) setManager(suggestedManager(r.backends)); })
      .catch(() => { /* no manager offer — the keychain path is unchanged */ });
    return () => { alive = false; };
  }, []);

  const found = providers.find(p => p.name === provider);
  const selected = found && savedHere.has(found.name) ? { ...found, isConfigured: true } : found;
  const managerKind: SourceKind = manager?.id === 'bitwarden' ? 'bitwarden' : 'onepassword';
  const refSpec = useReference ? buildSpec(managerKind, reference) : null;
  const typed = useReference ? !!refSpec && 'spec' in refSpec && !!refSpec.spec : key.trim().length > 0;
  const canTest = !!selected && (typed || selected.isConfigured);

  const pick = (name: string) => {
    generation.current += 1;
    setProvider(name);
    // Never carry a credential across providers.
    setKey(''); setReference(''); setUseReference(false);
    setPhase('idle'); setMessage(''); setShowHelp(false);
  };

  const test = async () => {
    if (!selected || !canTest) return false;
    const gen = ++generation.current;
    const current = () => generation.current === gen;
    setPhase('testing');
    setMessage('');
    try {
      if (useReference && refSpec && 'spec' in refSpec && refSpec.spec) {
        // Prove the reference resolves before committing to it.
        const probe = await api.testSecretSource(selected.secretKey, refSpec.spec);
        if (!current()) return false;
        if (!probe.ok) throw new Error(probe.error || `Couldn't read that reference from ${manager?.displayName ?? 'your password manager'}.`);
        await api.setSecretSource(selected.secretKey, refSpec.spec);
      } else if (key.trim()) {
        await api.upsertConfig(selected.secretKey, key.trim(), true);
      }
      if (!current()) return false;
      if (selected.defaultModel) await api.setProvider(selected.name, selected.defaultModel);
      else await api.upsertConfig('GOOSE_PROVIDER', selected.name);
      if (!current()) return false;
      const result = await api.checkProvider(selected.name);
      if (!current()) return false;
      const verdict = describeCheck(result);
      setPhase(verdict.tone === 'ok' ? 'ok' : 'inconclusive');
      setMessage(verdict.text);
      // The key is stored; it does not need to live in this component any more.
      setKey(''); setReference('');
      setSavedHere(prev => new Set(prev).add(selected.name));
      return true;
    } catch (e) {
      if (!current()) return false;
      console.error('Provider setup failed:', safeWizardError(e, 'provider setup failed'));
      setPhase('error');
      setMessage(friendlyCheckError(e instanceof Error ? e.message : String(e)));
      return false;
    }
  };

  const passed = phase === 'ok' || phase === 'inconclusive';
  const primary: StepPrimary = passed
    ? { label: 'Continue' }
    : { label: typed || !selected?.isConfigured ? 'Test key' : 'Test saved key', disabled: !canTest, pending: phase === 'testing', pendingLabel: 'Testing…' };

  // Hand the footer what it should do; the step's Enter runs the same thing.
  const run = passed ? onDone : test;
  const runRef = useRef(run);
  runRef.current = run;
  const label = primary.label; const disabled = !!primary.disabled; const pending = !!primary.pending;
  const pendingLabel = primary.pendingLabel;
  useEffect(() => {
    onAction({ primary: { label, disabled, pending, pendingLabel }, run: () => runRef.current() });
  }, [label, disabled, pending, pendingLabel, onAction]);
  useEffect(() => () => onAction(null), [onAction]);

  if (providers.length === 0) {
    return <StatusLine tone="warn">No key-based providers are available. Choose another option, or add one later in Settings → Models.</StatusLine>;
  }

  return (
    <Panel>
      <div style={{ display: 'flex', flexDirection: 'column', gap: space.xl }}>
        <div>
          <FieldLabel htmlFor={`${keyId}-provider`}>Provider</FieldLabel>
          <Select
            id={`${keyId}-provider`}
            ariaLabel="Provider"
            value={provider}
            onChange={pick}
            placeholder="Choose a provider"
            options={providers.map(p => ({ value: p.name, label: p.displayName, note: p.isConfigured ? 'Key saved' : undefined }))}
          />
        </div>

        {selected && (
          <div>
            <FieldLabel htmlFor={keyId}>{useReference ? `${manager?.displayName} reference` : 'API key'}</FieldLabel>
            {useReference ? (
              <Input id={keyId} value={reference} onChange={v => { setReference(v); setPhase('idle'); }}
                placeholder={REFERENCE_HINTS[managerKind].placeholder} describedBy={noteId} autoFocus />
            ) : (
              <Input id={keyId} type="password" value={key} onChange={v => { setKey(v); if (phase !== 'testing') setPhase('idle'); }}
                placeholder={selected.isConfigured ? 'Key saved. Paste a new one to replace it.' : `Paste your ${selected.displayName} API key`}
                describedBy={noteId} invalid={phase === 'error'} autoFocus />
            )}
            <div id={noteId} style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.textMuted, lineHeight: 1.5, marginTop: space.sm }}>
              {useReference
                ? `${REFERENCE_HINTS[managerKind].help} The key stays in ${manager?.displayName}; nothing is written to the keychain.`
                : 'Stored in your macOS keychain. macOS may ask once to let Permagent use it. Choose Always Allow.'}
            </div>
          </div>
        )}

        {phase === 'testing' && <StatusLine tone="info">Checking the key with {selected?.displayName}…</StatusLine>}
        {(phase === 'ok' || phase === 'inconclusive') && <StatusLine tone={phase === 'ok' ? 'ok' : 'warn'}>{message}</StatusLine>}
        {phase === 'error' && <StatusLine tone="error">{message}</StatusLine>}

        {selected && (
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.xl }}>
            {selected.setupSteps.length > 0 && (
              <GhostLink onClick={() => setShowHelp(s => !s)}>{showHelp ? 'Hide key steps' : "Don't have a key?"}</GhostLink>
            )}
            {manager && (
              <GhostLink onClick={() => { setUseReference(u => !u); setPhase('idle'); setMessage(''); }}>
                {useReference ? 'Paste a key instead' : `Use a ${manager.displayName} reference`}
              </GhostLink>
            )}
          </div>
        )}

        {selected && showHelp && (
          <ol style={{ margin: 0, paddingLeft: space.xxxl, display: 'flex', flexDirection: 'column', gap: space.xs }}>
            {selected.setupSteps.map((s, i) => {
              const part = splitStepLink(s);
              return (
                <li key={i} style={{ fontFamily: font.body, fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.5 }}>
                  {part.before}
                  {part.url && (
                    <a href={part.url} target="_blank" rel="noopener noreferrer" style={{ color: colors.cyan }}>{part.url}</a>
                  )}
                  {part.after}
                </li>
              );
            })}
          </ol>
        )}
      </div>
    </Panel>
  );
}

// ── Local (Ollama) ─────────────────────────────────────────────────────

type LocalState = 'checking' | 'down' | 'starting' | 'up' | 'error';

interface Pull { model: string; percent: number; status: string }

function gb(bytes: number): string {
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}

function LocalPanel({ onAction, onDone }: {
  onAction: (a: PanelAction | null) => void;
  onDone: () => void;
}) {
  const { colors } = useTheme();
  const [state, setState] = useState<LocalState>('checking');
  const [error, setError] = useState('');
  const [installed, setInstalled] = useState<Array<{ name: string; size: number }>>([]);
  const [selected, setSelected] = useState('');
  const [suggested, setSuggested] = useState<{ model: string; sizeGB: number } | null>(null);
  const [pull, setPull] = useState<Pull | null>(null);
  const [pullNote, setPullNote] = useState('');
  const [phase, setPhase] = useState<'idle' | 'testing' | 'ok' | 'error'>('idle');
  const [message, setMessage] = useState('');
  const abortRef = useRef<(() => void) | null>(null);
  const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; abortRef.current?.(); }; }, []);

  const refresh = useCallback(async () => {
    try {
      const status = await api.getOllamaStatus();
      if (!alive.current) return false;
      if (!status.reachable) { setState(s => (s === 'starting' ? s : 'down')); return false; }
      setInstalled(status.installed.map(m => ({ name: m.name, size: m.size })));
      setSelected(cur => cur && status.installed.some(m => m.name === cur) ? cur : status.installed[0]?.name ?? '');
      setState('up');
      return true;
    } catch (e) {
      if (!alive.current) return false;
      setError(e instanceof Error ? e.message : String(e));
      setState('error');
      return false;
    }
  }, []);

  useEffect(() => {
    void refresh();
    api.getSystemInfo()
      .then(info => {
        const tier = recommendModel(info.total_ram_bytes || 0);
        if (alive.current && tier) setSuggested({ model: tier.model, sizeGB: tier.downloadSizeGB });
      })
      .catch(() => { /* no size hint — the download offer just hides */ });
  }, [refresh]);

  // Start only when asked — never as a side effect of opening the step.
  const start = async () => {
    setState('starting');
    try { await api.startOllama(); } catch { /* not installed — polling below reports it */ }
    for (let i = 0; i < 8 && alive.current; i += 1) {
      await new Promise(r => setTimeout(r, 2000));
      if (await refresh()) return true;
    }
    if (alive.current) setState('down');
    return false;
  };

  const download = async (model: string) => {
    setPull({ model, percent: 0, status: 'Starting download…' });
    setPullNote('');
    const job = api.pullOllamaModel(model, data => {
      if (!alive.current) return;
      setPull(p => p && {
        ...p,
        status: data.status || p.status,
        percent: data.total && data.completed ? Math.round((data.completed / data.total) * 100) : p.percent,
      });
    });
    abortRef.current = job.abort;
    try {
      await job.promise;
      if (!alive.current) return;
      setPull(null);
      await refresh();
      setSelected(model);
    } catch (e) {
      if (!alive.current) return;
      setPull(null);
      setPullNote((e as Error).name === 'AbortError'
        ? 'Download stopped. Ollama keeps what it fetched, so starting again picks up where it left off.'
        : `Download failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      abortRef.current = null;
    }
  };
  const cancel = () => abortRef.current?.();

  const test = async () => {
    if (!selected) return false;
    setPhase('testing');
    setMessage('');
    try {
      await api.setProvider(LOCAL_RUNTIME_PROVIDER, selected);
      const result = await api.checkProvider(LOCAL_RUNTIME_PROVIDER);
      if (!alive.current) return false;
      if (result && !result.live) throw new Error(result.note || 'Ollama did not list any models.');
      setPhase('ok');
      setMessage(`Ready · ${selected} runs on this Mac`);
      return true;
    } catch (e) {
      if (!alive.current) return false;
      setPhase('error');
      setMessage(friendlyCheckError(e instanceof Error ? e.message : String(e)));
      return false;
    }
  };

  const primary: StepPrimary = phase === 'ok'
    ? { label: 'Continue' }
    : { label: 'Use this model', disabled: !selected || !!pull || state !== 'up', pending: phase === 'testing', pendingLabel: 'Checking…' };
  const run = phase === 'ok' ? onDone : test;
  const runRef = useRef(run);
  runRef.current = run;
  const { label, pendingLabel } = primary; const disabled = !!primary.disabled; const pending = !!primary.pending;
  useEffect(() => {
    onAction({ primary: { label, disabled, pending, pendingLabel }, run: () => runRef.current() });
  }, [label, disabled, pending, pendingLabel, onAction]);
  useEffect(() => () => onAction(null), [onAction]);

  const suggestionInstalled = !!suggested && installed.some(m => m.name === suggested.model || m.name === `${suggested.model}:latest`);

  return (
    <Panel>
      <div style={{ display: 'flex', flexDirection: 'column', gap: space.xl }}>
        {state === 'checking' && <StatusLine tone="info">Looking for Ollama on this Mac…</StatusLine>}
        {state === 'starting' && <StatusLine tone="info">Starting Ollama…</StatusLine>}
        {state === 'error' && <StatusLine tone="error">Couldn't check for Ollama ({error}).</StatusLine>}
        {state === 'down' && (
          <>
            <StatusLine tone="warn">Ollama isn't running on this Mac. It runs models locally and is free.</StatusLine>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.md }}>
              <SecondaryButton onClick={start}>Start Ollama</SecondaryButton>
              <SecondaryButton onClick={() => { window.open('https://ollama.com/download', '_blank', 'noopener,noreferrer'); }}>Get Ollama ↗</SecondaryButton>
              <GhostLink onClick={refresh}>Check again</GhostLink>
            </div>
          </>
        )}

        {state === 'up' && installed.length > 0 && (
          <div role="radiogroup" aria-label="Installed models" style={{ display: 'flex', flexDirection: 'column', gap: space.xs }}>
            <FieldLabel>Installed models</FieldLabel>
            {installed.map(m => (
              <label key={m.name} style={{
                display: 'flex', alignItems: 'center', gap: space.lg, cursor: 'pointer',
                padding: `${space.md}px ${space.lg}px`, borderRadius: radius.md,
                background: selected === m.name ? colors.cyanSoft : 'transparent',
                transition: `background ${duration.fast}ms ${ease.out}`,
              }}>
                <input type="radio" name="local-model" checked={selected === m.name}
                  onChange={() => { setSelected(m.name); setPhase('idle'); }} style={{ accentColor: colors.cyan }} />
                <span style={{ flex: 1, fontFamily: font.mono, fontSize: textSize.small, color: colors.text }}>{m.name}</span>
                <span style={{ fontFamily: font.body, fontSize: textSize.caption, color: colors.textMuted }}>{gb(m.size)}</span>
              </label>
            ))}
          </div>
        )}

        {state === 'up' && installed.length === 0 && !pull && (
          <StatusLine tone="warn">Ollama is running but has no models yet. Download one to continue.</StatusLine>
        )}

        {state === 'up' && suggested && !suggestionInstalled && !pull && (
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: space.xl, flexWrap: 'wrap' }}>
            <span style={{ fontFamily: font.body, fontSize: textSize.small, color: colors.textMuted, lineHeight: 1.5, flex: 1, minWidth: 200 }}>
              Suggested for this Mac's memory: <span style={{ fontFamily: font.mono, color: colors.text }}>{suggested.model}</span>, about {suggested.sizeGB} GB.
            </span>
            <SecondaryButton onClick={() => download(suggested.model)}>Download</SecondaryButton>
          </div>
        )}

        {pull && (
          <div>
            <div style={{ display: 'flex', justifyContent: 'space-between', gap: space.md, marginBottom: space.sm }}>
              <span style={{ fontFamily: font.body, fontSize: textSize.small, color: colors.text }}>Downloading {pull.model}</span>
              <span style={{ fontFamily: font.mono, fontSize: textSize.caption, color: colors.cyan }}>{pull.percent}%</span>
            </div>
            <div role="progressbar" aria-label={`Downloading ${pull.model}`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={pull.percent}
              style={{ height: 6, borderRadius: radius.pill, background: colors.fillActive, overflow: 'hidden' }}>
              <div style={{ width: `${pull.percent}%`, height: '100%', background: colors.cyan, transition: `width ${duration.slow}ms ${ease.out}` }} />
            </div>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginTop: space.sm, gap: space.md }}>
              <span style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textMuted, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{pull.status}</span>
              <SecondaryButton onClick={cancel}>Cancel</SecondaryButton>
            </div>
          </div>
        )}
        {pullNote && <StatusLine tone="warn">{pullNote}</StatusLine>}

        {phase === 'testing' && <StatusLine tone="info">Checking {selected}…</StatusLine>}
        {phase === 'ok' && <StatusLine tone="ok">{message}</StatusLine>}
        {phase === 'error' && <StatusLine tone="error">{message}</StatusLine>}
      </div>
    </Panel>
  );
}

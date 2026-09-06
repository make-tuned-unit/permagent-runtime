import { useState, useEffect, useRef } from 'react';
import { FiCheckCircle } from 'react-icons/fi';
import { ease, font, radius, type ThemeColors, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { PrimaryButton, GhostLink, Glass, Particles, Select, type SelectOption } from './atoms';
import { Mobius } from '../mobius/Mobius';
import { api } from '../../lib/api';
import { recommendModel, compatibleModels, MODEL_TIERS, type ModelTier, MIN_RAM_GB } from '../../lib/modelTiers';
import { safeWizardError } from './wizardErrors';

type Phase = 'scanning' | 'recommend' | 'downloading' | 'skipped' | 'ready' | 'error';

interface HardwareInfo {
  cpuBrand: string;
  ramGB: number;
  diskFreeGB: number;
  totalRamBytes: number;
}

interface Props {
  active?: boolean;
  onAdvance: () => void;
  onBack: () => void;
}

function formatGB(bytes: number): number {
  return Math.round(bytes / (1024 * 1024 * 1024));
}

export function MomentHardware({ active = true, onAdvance }: Props) {
  const { colors, theme } = useTheme();
  // Progress track: a white wash is invisible on silver — flip to graphite.
  const trackVeil = theme === 'silver' ? 'rgba(30,37,48,0.08)' : 'rgba(255,255,255,0.06)';
  const [phase, setPhase] = useState<Phase>('scanning');
  const [hardware, setHardware] = useState<HardwareInfo | null>(null);
  const [recommended, setRecommended] = useState<ModelTier | null>(null);
  const [selectedModel, setSelectedModel] = useState('');
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [ollamaReachable, setOllamaReachable] = useState<boolean | null>(null);
  const [modelInstalled, setModelInstalled] = useState(false);
  const [pullProgress, setPullProgress] = useState(0);
  const [pullStatus, setPullStatus] = useState('');
  const [errorMsg, setErrorMsg] = useState('');
  const [scanNonce, setScanNonce] = useState(0);
  const [ollamaStarting, setOllamaStarting] = useState(false);
  const autoStartTried = useRef(false);
  const pullAbortRef = useRef<(() => void) | null>(null);
  const activeRef = useRef(active);
  activeRef.current = active;
  const scanGenerationRef = useRef(0);
  const statusGenerationRef = useRef(0);
  const operationGenerationRef = useRef(0);

  // Phase 1: Scan hardware (re-runs when Retry bumps the nonce)
  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    const generation = ++scanGenerationRef.current;
    const current = () => !cancelled && activeRef.current && scanGenerationRef.current === generation;
    (async () => {
      try {
        const info = await api.getSystemInfo();
        if (!current()) return;
        const ramBytes = info.total_ram_bytes || 0;
        const hw: HardwareInfo = {
          cpuBrand: info.cpu_brand || info.architecture || 'Unknown',
          ramGB: formatGB(ramBytes),
          diskFreeGB: formatGB(info.disk_free_bytes || 0),
          totalRamBytes: ramBytes,
        };
        setHardware(hw);

        const rec = recommendModel(ramBytes);
        setRecommended(rec);
        // Re-entry/retry must not erase a model the user selected in the
        // advanced picker while this mounted Moment was temporarily hidden.
        if (rec) setSelectedModel(current => current || rec.model);

        // Check Ollama status
        try {
          const status = await api.getOllamaStatus();
          if (!current()) return;
          setOllamaReachable(status.reachable);
          if (rec && status.installed.some(m => m.name.startsWith(rec.model.split(':')[0]) && m.name.includes(rec.model.split(':')[1]))) {
            setModelInstalled(true);
          }
        } catch {
          if (!current()) return;
          setOllamaReachable(false);
        }

        if (!current()) return;
        setPhase('recommend');
      } catch (e) {
        if (current()) {
          setErrorMsg(e instanceof Error ? e.message : 'Failed to read system info');
          setPhase('error');
        }
      }
    })();
    return () => {
      cancelled = true;
      if (scanGenerationRef.current === generation) scanGenerationRef.current++;
    };
  }, [active, scanNonce]);

  // One-click (#381): when Ollama is unreachable, try to start it ourselves
  // (installed-but-not-running is the common case) and poll status — the user
  // never has to hit a re-check button. The install panel only appears once
  // the auto-start window has passed with no server coming up.
  useEffect(() => {
    if (!active || phase !== 'recommend' || ollamaReachable !== false) return;
    let cancelled = false;
    let polls = 0;
    if (!autoStartTried.current) {
      autoStartTried.current = true;
      setOllamaStarting(true);
      api.startOllama().catch(() => { /* not installed — polling will time out */ });
    }
    const timer = setInterval(async () => {
      polls += 1;
      try {
        const status = await api.getOllamaStatus();
        if (cancelled) return;
        if (status.reachable) {
          setOllamaReachable(true);
          setOllamaStarting(false);
          return;
        }
      } catch { /* keep polling */ }
      if (!cancelled && polls >= 8) setOllamaStarting(false); // ~16s: assume not installed
    }, 2000);
    return () => { cancelled = true; clearInterval(timer); };
  }, [active, phase, ollamaReachable]);

  // Ready: auto-advance after a beat — the success state should not need a
  // click, but the button stays for the impatient.
  useEffect(() => {
    if (!active || phase !== 'ready') return;
    const t = setTimeout(() => {
      if (activeRef.current) onAdvance();
    }, 1600);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, phase]);

  // Check if selected model is installed when selection changes
  useEffect(() => {
    if (!active || !selectedModel || ollamaReachable === null) return;
    let cancelled = false;
    const generation = ++statusGenerationRef.current;
    const current = () => !cancelled && activeRef.current && statusGenerationRef.current === generation;
    (async () => {
      try {
        const status = await api.getOllamaStatus();
        if (!current()) return;
        const installed = status.installed.some(m => {
          const sel = selectedModel.split(':');
          return m.name.startsWith(sel[0]) && m.name.includes(sel[1] || '');
        });
        setModelInstalled(installed);
      } catch { /* ignore */ }
    })();
    return () => {
      cancelled = true;
      if (statusGenerationRef.current === generation) statusGenerationRef.current++;
    };
  }, [active, selectedModel, ollamaReachable]);

  // A Back navigation must not leave a native model pull or auto-start poll
  // running in an invisible step. Preserve the selected model/progress, but
  // return to the recommendation screen so re-entry has an honest restart
  // affordance instead of a completed-looking orphaned promise.
  useEffect(() => {
    if (active) return;
    pullAbortRef.current?.();
    pullAbortRef.current = null;
    operationGenerationRef.current++;
    setOllamaStarting(false);
    setPhase(current => current === 'downloading' ? 'recommend' : current);
  }, [active]);

  const handleSetup = async () => {
    if (!activeRef.current || !ollamaReachable) return;
    const operation = ++operationGenerationRef.current;
    const current = () => activeRef.current && operationGenerationRef.current === operation;

    if (modelInstalled) {
      // Already have the model, just apply config
      if (await applyConfig(selectedModel, true, current) && current()) setPhase('ready');
      return;
    }

    // Start download
    setPhase('downloading');
    setPullProgress(0);
    setPullStatus('Starting download...');

    let abort: (() => void) | undefined;
    try {
      const pull = api.pullOllamaModel(selectedModel, (data) => {
        if (!current()) return;
        if (data.status) setPullStatus(data.status);
        if (data.total && data.completed) {
          setPullProgress(Math.round((data.completed / data.total) * 100));
        }
      });
      abort = pull.abort;
      pullAbortRef.current = abort;
      await pull.promise;
      if (await applyConfig(selectedModel, true, current) && current()) setPhase('ready');
    } catch (e) {
      if (current() && (e as Error).name !== 'AbortError') {
        setErrorMsg(e instanceof Error ? e.message : 'Download failed');
        setPhase('error');
      }
    } finally {
      if (pullAbortRef.current === abort) pullAbortRef.current = null;
    }
  };

  const handleSkip = async () => {
    if (!activeRef.current) return;
    const operation = ++operationGenerationRef.current;
    const current = () => activeRef.current && operationGenerationRef.current === operation;
    await applyConfig('', false, current);
    if (current()) setPhase('skipped');
  };

  /** Persist the Librarian schedule. Setup failures surface as the error
   *  phase (a silent catch here would let the wizard claim the Librarian is
   *  configured when it isn't); the skip path stays best-effort. */
  async function applyConfig(model: string, enabled: boolean, isCurrent: () => boolean): Promise<boolean> {
    if (!isCurrent()) return false;
    try {
      // Fetch current schedule to preserve other fields
      const schedule = await api.getLibrarianSchedule();
      if (!isCurrent()) return false;
      await api.setLibrarianSchedule({
        ...schedule,
        enabled,
        model: model || schedule.model,
      });
      return isCurrent();
    } catch (e) {
      console.error('Failed to save Librarian config:', safeWizardError(e, 'Librarian configuration save failed'));
      if (enabled && isCurrent()) {
        setErrorMsg(e instanceof Error ? e.message : 'Failed to save Librarian settings');
        setPhase('error');
      }
      return false;
    }
  }

  const selectedTier = MODEL_TIERS.find(t => t.model === selectedModel);

  // Build model dropdown options
  const modelOptions: SelectOption[] = hardware
    ? compatibleModels(hardware.totalRamBytes).map(t => ({
        value: t.model,
        label: `${t.model} — ${t.label}`,
        note: `~${t.downloadSizeGB} GB`,
      }))
    : [];

  return (
    <div style={{
      display: 'flex', flexDirection: 'column', alignItems: 'center',
      justifyContent: 'center', height: '100%', position: 'relative',
    }}>
      <Particles density={16} />

      {/* ── SCANNING ────────────────────────────────────────── */}
      {phase === 'scanning' && (
        <>
          <Mobius size={140} state="calibrating" />
          <h1 style={h1Style(colors)}>Optimizing for your hardware</h1>
          <p style={subtitleStyle(colors)}>Scanning your system...</p>
        </>
      )}

      {/* ── RECOMMENDATION ──────────────────────────────────── */}
      {phase === 'recommend' && hardware && (
        <>
          <Mobius size={120} state="idle" />
          <h1 style={h1Style(colors)}>Optimizing for your hardware</h1>

          {/* Hardware summary */}
          <Glass r={12} padding={16} style={{ width: 400, marginBottom: 20 }}>
            <div style={{ display: 'flex', gap: 20, justifyContent: 'center' }}>
              <HwStat label="CPU" value={hardware.cpuBrand.replace('Apple ', '')} />
              <HwStat label="RAM" value={`${hardware.ramGB} GB`} />
              <HwStat label="Free" value={`${hardware.diskFreeGB} GB`} />
            </div>
          </Glass>

          {hardware.ramGB < MIN_RAM_GB ? (
            /* Not enough RAM for any local model */
            <>
              <p style={{ ...subtitleStyle(colors), maxWidth: 400 }}>
                Your system has {hardware.ramGB} GB of RAM — not enough to run a local model comfortably.
                The Librarian will be disabled, but everything else in Permagent works normally.
              </p>
              <PrimaryButton onClick={async () => { await handleSkip(); }} full style={{ width: 360, marginTop: 8 }}>
                Continue
              </PrimaryButton>
            </>
          ) : (
            /* Has enough RAM — show recommendation + opt-out */
            <>
              {recommended && (
                <p style={{ ...subtitleStyle(colors), maxWidth: 420 }}>
                  We recommend <span style={{ color: colors.cyan, fontWeight: 600 }}>{recommended.model}</span> for
                  your hardware — your {hardware.ramGB} GB of unified memory can run it comfortably.
                </p>
              )}

              {showAdvanced && (
                <div style={{ width: 360, marginBottom: 12 }}>
                  <Select value={selectedModel} onChange={setSelectedModel} options={modelOptions} />
                </div>
              )}

              {!showAdvanced && (
                <GhostLink onClick={() => setShowAdvanced(true)} style={{ marginBottom: 12 }}>
                  Use a different model
                </GhostLink>
              )}

              {/* PATH A: one-click setup */}
              <div style={{ width: 400, display: 'flex', flexDirection: 'column', gap: 10 }}>
                <PrimaryButton onClick={handleSetup} disabled={!ollamaReachable} full>
                  {modelInstalled
                    ? 'Enable the Librarian'
                    : `Install ${selectedModel}${selectedTier ? ` — ${selectedTier.downloadSizeGB} GB, free` : ''}`}
                </PrimaryButton>
                <p style={fineprint(colors)}>
                  Your agent's memory will be enriched with descriptions,
                  entity connections, and vocabulary bridging. The Brain
                  learns and improves over time.
                </p>

                {ollamaReachable === false && ollamaStarting && (
                  <p style={{ ...fineprint(colors), color: colors.textMuted, marginTop: 4 }}>
                    Starting the local model runtime…
                  </p>
                )}

                {ollamaReachable === false && !ollamaStarting && (
                  <Glass r={10} padding={14} style={{ marginTop: 4 }}>
                    <p style={{ ...fineprint(colors), margin: '0 0 8px' }}>
                      One quick install first: Ollama runs the model on your
                      machine — free and private. We'll pick things up here
                      automatically the moment it's ready.
                    </p>
                    <GhostLink onClick={() => window.open('https://ollama.com/download', '_blank')}>
                      Get Ollama
                    </GhostLink>
                  </Glass>
                )}

                {/* PATH B: Skip */}
                <div style={{ borderTop: `1px solid ${colors.border}`, paddingTop: 14, marginTop: 6 }}>
                  <GhostLink onClick={handleSkip} style={{ display: 'block', textAlign: 'center' }}>
                    Skip for now
                  </GhostLink>
                  <p style={{ ...fineprint(colors), marginTop: 6 }}>
                    Your agent still works fully — chat, tasks, browser, everything functions.
                    But memories won't be described or enriched. You can enable the Librarian
                    later in Settings.
                  </p>
                </div>
              </div>
            </>
          )}
        </>
      )}

      {/* ── DOWNLOADING ─────────────────────────────────────── */}
      {phase === 'downloading' && (
        <>
          <Mobius size={120} state="calibrating" />
          <h1 style={h1Style(colors)}>Downloading {selectedModel}</h1>
          <div style={{ width: 360, marginTop: 16 }}>
            {/* Progress bar */}
            <div style={{
              width: '100%', height: 6, borderRadius: radius.pill,
              background: trackVeil, overflow: 'hidden',
            }}>
              <div style={{
                width: `${pullProgress}%`, height: '100%', borderRadius: radius.pill,
                background: `linear-gradient(90deg, ${colors.cyan}, ${colors.purple})`,
                boxShadow: `0 0 12px ${colors.cyanGlow}`,
                transition: `width 300ms ${ease.out}`,
              }} />
            </div>
            <div style={{ display: 'flex', justifyContent: 'space-between', marginTop: 8 }}>
              <span style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textMuted }}>
                {pullStatus.length > 50 ? pullStatus.slice(0, 50) + '...' : pullStatus}
              </span>
              <span style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.cyan }}>
                {pullProgress}%
              </span>
            </div>
          </div>
        </>
      )}

      {/* ── SKIPPED ──────────────────────────────────────────── */}
      {phase === 'skipped' && (
        <>
          <Mobius size={120} state="idle" />
          <h1 style={h1Style(colors)}>Librarian disabled</h1>
          <p style={{ ...subtitleStyle(colors), maxWidth: 380 }}>
            You can enable it anytime in Settings → Librarian.
          </p>
          <PrimaryButton onClick={onAdvance} full style={{ width: 360, marginTop: 16 }}>
            Continue
          </PrimaryButton>
        </>
      )}

      {/* ── READY ────────────────────────────────────────────── */}
      {phase === 'ready' && (
        <>
          <Mobius size={120} state="idle" />
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginTop: 8 }}>
            <FiCheckCircle size={22} color={colors.cyan} />
            <h1 style={{ ...h1Style(colors), margin: 0 }}>Ready to go</h1>
          </div>
          <p style={{ ...subtitleStyle(colors), maxWidth: 380, marginTop: 10 }}>
            <span style={{ color: colors.cyan, fontWeight: 600 }}>{selectedModel}</span> is installed.
            The Librarian will enrich your agent's memory automatically.
          </p>
          <PrimaryButton onClick={onAdvance} full style={{ width: 360, marginTop: 20 }}>
            Continue
          </PrimaryButton>
          <p style={{ ...fineprint(colors), marginTop: 10 }}>Continuing automatically…</p>
        </>
      )}

      {/* ── ERROR ────────────────────────────────────────────── */}
      {phase === 'error' && (
        <>
          <Mobius size={120} state="idle" />
          <h1 style={h1Style(colors)}>Something went wrong</h1>
          <p style={{ ...subtitleStyle(colors), color: colors.danger, maxWidth: 380 }}>{errorMsg}</p>
          <div style={{ display: 'flex', gap: 12, marginTop: 16 }}>
            <GhostLink onClick={() => { setPhase('scanning'); setScanNonce(n => n + 1); }}>Retry</GhostLink>
            <GhostLink onClick={handleSkip}>Skip for now</GhostLink>
          </div>
        </>
      )}
    </div>
  );
}

function HwStat({ label, value }: { label: string; value: string }) {
  const { colors } = useTheme();
  return (
    <div style={{ textAlign: 'center' }}>
      <div style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textMuted, marginBottom: 4, textTransform: 'uppercase', letterSpacing: '0.06em' }}>
        {label}
      </div>
      <div style={{ fontFamily: font.body, fontSize: textSize.body, fontWeight: 600, color: colors.text }}>
        {value}
      </div>
    </div>
  );
}

const h1Style = (c: ThemeColors) => ({
  fontFamily: font.display, fontSize: 28, fontWeight: 700,
  color: c.text, margin: '24px 0 8px', letterSpacing: '-0.02em',
} as const);

const subtitleStyle = (c: ThemeColors) => ({
  fontFamily: font.body, fontSize: textSize.body, color: c.textMuted,
  marginBottom: 24, textAlign: 'center' as const, lineHeight: 1.6,
} as const);

const fineprint = (c: ThemeColors) => ({
  fontFamily: font.body, fontSize: textSize.caption, color: c.textDim,
  lineHeight: 1.5, textAlign: 'center' as const, margin: 0,
} as const);

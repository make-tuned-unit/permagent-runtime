import { useState, useEffect, useRef } from 'react';
import { FiCheckCircle } from 'react-icons/fi';
import { ease, font, radius, space, type ThemeColors, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { PrimaryButton, GhostLink, Glass, Particles, Select, type SelectOption } from './atoms';
import { Mobius } from '../mobius/Mobius';
import { api } from '../../lib/api';
import { recommendModel, compatibleModels, MODEL_TIERS, type ModelTier, MIN_RAM_GB } from '../../lib/modelTiers';

type Phase = 'scanning' | 'recommend' | 'downloading' | 'skipped' | 'ready' | 'error';

interface HardwareInfo {
  cpuBrand: string;
  ramGB: number;
  diskFreeGB: number;
  totalRamBytes: number;
}

interface Props {
  onAdvance: () => void;
  onBack: () => void;
}

function formatGB(bytes: number): number {
  return Math.round(bytes / (1024 * 1024 * 1024));
}

export function MomentHardware({ onAdvance }: Props) {
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

  // Phase 1: Scan hardware (re-runs when Retry bumps the nonce)
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const info = await api.getSystemInfo();
        if (cancelled) return;
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
        if (rec) setSelectedModel(rec.model);

        // Check Ollama status
        try {
          const status = await api.getOllamaStatus();
          setOllamaReachable(status.reachable);
          if (rec && status.installed.some(m => m.name.startsWith(rec.model.split(':')[0]) && m.name.includes(rec.model.split(':')[1]))) {
            setModelInstalled(true);
          }
        } catch {
          setOllamaReachable(false);
        }

        setPhase('recommend');
      } catch (e) {
        if (!cancelled) {
          setErrorMsg(e instanceof Error ? e.message : 'Failed to read system info');
          setPhase('error');
        }
      }
    })();
    return () => { cancelled = true; };
  }, [scanNonce]);

  // One-click (#381): when Ollama is unreachable, try to start it ourselves
  // (installed-but-not-running is the common case) and poll status — the user
  // never has to hit a re-check button. The install panel only appears once
  // the auto-start window has passed with no server coming up.
  useEffect(() => {
    if (phase !== 'recommend' || ollamaReachable !== false) return;
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
      if (polls >= 8) setOllamaStarting(false); // ~16s: assume not installed
    }, 2000);
    return () => { cancelled = true; clearInterval(timer); };
  }, [phase, ollamaReachable]);

  // Ready: auto-advance after a beat — the success state should not need a
  // click, but the button stays for the impatient.
  useEffect(() => {
    if (phase !== 'ready') return;
    const t = setTimeout(onAdvance, 1600);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase]);

  // Check if selected model is installed when selection changes
  useEffect(() => {
    if (!selectedModel || ollamaReachable === null) return;
    (async () => {
      try {
        const status = await api.getOllamaStatus();
        const installed = status.installed.some(m => {
          const sel = selectedModel.split(':');
          return m.name.startsWith(sel[0]) && m.name.includes(sel[1] || '');
        });
        setModelInstalled(installed);
      } catch { /* ignore */ }
    })();
  }, [selectedModel, ollamaReachable]);

  const handleSetup = async () => {
    if (!ollamaReachable) return;

    if (modelInstalled) {
      // Already have the model, just apply config
      if (await applyConfig(selectedModel, true)) setPhase('ready');
      return;
    }

    // Start download
    setPhase('downloading');
    setPullProgress(0);
    setPullStatus('Starting download...');

    const { promise, abort } = api.pullOllamaModel(selectedModel, (data) => {
      if (data.status) setPullStatus(data.status);
      if (data.total && data.completed) {
        setPullProgress(Math.round((data.completed / data.total) * 100));
      }
    });
    pullAbortRef.current = abort;

    try {
      await promise;
      if (await applyConfig(selectedModel, true)) setPhase('ready');
    } catch (e) {
      if ((e as Error).name !== 'AbortError') {
        setErrorMsg(e instanceof Error ? e.message : 'Download failed');
        setPhase('error');
      }
    }
  };

  const handleSkip = async () => {
    await applyConfig('', false);
    setPhase('skipped');
  };

  /** Persist the Librarian schedule. Setup failures surface as the error
   *  phase (a silent catch here would let the wizard claim the Librarian is
   *  configured when it isn't); the skip path stays best-effort. */
  async function applyConfig(model: string, enabled: boolean): Promise<boolean> {
    try {
      // Fetch current schedule to preserve other fields
      const current = await api.getLibrarianSchedule();
      await api.setLibrarianSchedule({
        ...current,
        enabled,
        model: model || current.model,
      });
      return true;
    } catch (e) {
      console.error('Failed to save Librarian config:', e);
      if (enabled) {
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
          <Glass r={12} padding={16} style={{ width: 400, marginBottom: space.xxxl }}>
            <div style={{ display: 'flex', gap: space.xxxl, justifyContent: 'center' }}>
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
              <PrimaryButton onClick={async () => { await handleSkip(); }} full style={{ width: 360, marginTop: space.md }}>
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
                <div style={{ width: 360, marginBottom: space.xl }}>
                  <Select value={selectedModel} onChange={setSelectedModel} options={modelOptions} />
                </div>
              )}

              {!showAdvanced && (
                <GhostLink onClick={() => setShowAdvanced(true)} style={{ marginBottom: space.xl }}>
                  Use a different model
                </GhostLink>
              )}

              {/* PATH A: one-click setup */}
              <div style={{ width: 400, display: 'flex', flexDirection: 'column', gap: space.lg }}>
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
                  <p style={{ ...fineprint(colors), color: colors.textMuted, marginTop: space.xs }}>
                    Starting the local model runtime…
                  </p>
                )}

                {ollamaReachable === false && !ollamaStarting && (
                  <Glass r={10} padding={14} style={{ marginTop: space.xs }}>
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
                <div style={{ borderTop: `1px solid ${colors.border}`, paddingTop: 14, marginTop: space.sm }}>
                  <GhostLink onClick={handleSkip} style={{ display: 'block', textAlign: 'center' }}>
                    Skip for now
                  </GhostLink>
                  <p style={{ ...fineprint(colors), marginTop: space.sm }}>
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
          <div style={{ width: 360, marginTop: space.xxl }}>
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
            <div style={{ display: 'flex', justifyContent: 'space-between', marginTop: space.md }}>
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
          <PrimaryButton onClick={onAdvance} full style={{ width: 360, marginTop: space.xxl }}>
            Continue
          </PrimaryButton>
        </>
      )}

      {/* ── READY ────────────────────────────────────────────── */}
      {phase === 'ready' && (
        <>
          <Mobius size={120} state="idle" />
          <div style={{ display: 'flex', alignItems: 'center', gap: space.lg, marginTop: space.md }}>
            <FiCheckCircle size={22} color={colors.cyan} />
            <h1 style={{ ...h1Style(colors), margin: 0 }}>Ready to go</h1>
          </div>
          <p style={{ ...subtitleStyle(colors), maxWidth: 380, marginTop: space.lg }}>
            <span style={{ color: colors.cyan, fontWeight: 600 }}>{selectedModel}</span> is installed.
            The Librarian will enrich your agent's memory automatically.
          </p>
          <PrimaryButton onClick={onAdvance} full style={{ width: 360, marginTop: space.xxxl }}>
            Continue
          </PrimaryButton>
          <p style={{ ...fineprint(colors), marginTop: space.lg }}>Continuing automatically…</p>
        </>
      )}

      {/* ── ERROR ────────────────────────────────────────────── */}
      {phase === 'error' && (
        <>
          <Mobius size={120} state="idle" />
          <h1 style={h1Style(colors)}>Something went wrong</h1>
          <p style={{ ...subtitleStyle(colors), color: colors.danger, maxWidth: 380 }}>{errorMsg}</p>
          <div style={{ display: 'flex', gap: space.xl, marginTop: space.xxl }}>
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
      <div style={{ fontFamily: font.mono, fontSize: textSize.micro, color: colors.textMuted, marginBottom: space.xs, textTransform: 'uppercase', letterSpacing: '0.06em' }}>
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
  marginBottom: space.huge, textAlign: 'center' as const, lineHeight: 1.6,
} as const);

const fineprint = (c: ThemeColors) => ({
  fontFamily: font.body, fontSize: textSize.caption, color: c.textDim,
  lineHeight: 1.5, textAlign: 'center' as const, margin: 0,
} as const);

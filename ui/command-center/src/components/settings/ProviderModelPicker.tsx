import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { FiCheck, FiChevronDown, FiLoader } from 'react-icons/fi';
import { api } from '../../lib/api';
import { useCommandCenter, type ProviderInfo } from '../../lib/store';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export type ProviderModelValue = { provider: string | null; model: string | null };

type Props = {
  value: ProviderModelValue;
  onChange: (value: { provider: string; model: string }) => void;
  onUseSession?: () => void;
  sessionActive?: boolean;
  compact?: boolean;
  'aria-label'?: string;
};

/** Provider-first model chooser shared by Chat and role defaults. */
export function ProviderModelPicker({ value, onChange, onUseSession, sessionActive, compact, 'aria-label': ariaLabel }: Props) {
  const { colors } = useTheme();
  const providers = useCommandCenter(s => s.providers);
  const loadProviders = useCommandCenter(s => s.loadProviders);
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [models, setModels] = useState<Record<string, string[]>>({});
  const [modelErrors, setModelErrors] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => { if (providers.length === 0) void loadProviders(); }, [providers.length, loadProviders]);
  const configured = useMemo(() => providers.filter(p => p.isConfigured), [providers]);
  const selectedProvider = providers.find(p => p.name === value.provider);
  const label = value.provider && value.model ? `${selectedProvider?.displayName ?? value.provider} / ${value.model}` : (sessionActive ? 'Use session model' : 'Select a provider and model');
  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return configured.filter(p => !q || p.name.toLowerCase().includes(q) || p.displayName.toLowerCase().includes(q) || (models[p.name] ?? p.knownModels).some(m => m.toLowerCase().includes(q)));
  }, [configured, query, models]);
  useEffect(() => {
    const q = query.trim().toLowerCase();
    if (!q || expanded) return;
    const match = configured.find(p => p.name.toLowerCase().includes(q) || p.displayName.toLowerCase().includes(q) || p.knownModels.some(m => m.toLowerCase().includes(q)));
    if (match) setExpanded(match.name);
  }, [query, configured, expanded]);
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === 'Escape') setOpen(false); };
    document.addEventListener('pointerdown', onPointerDown);
    document.addEventListener('keydown', onKeyDown);
    return () => { document.removeEventListener('pointerdown', onPointerDown); document.removeEventListener('keydown', onKeyDown); };
  }, [open]);

  const expand = async (provider: ProviderInfo) => {
    if (expanded === provider.name) { setExpanded(null); return; }
    setExpanded(provider.name);
    if (models[provider.name]) return;
    setLoading(provider.name);
    try {
      const fetched = await api.getProviderModels(provider.name);
      setModels(prev => ({ ...prev, [provider.name]: fetched.length ? fetched : provider.knownModels }));
      setModelErrors(prev => {
        const next = { ...prev };
        delete next[provider.name];
        return next;
      });
    } catch {
      setModels(prev => ({ ...prev, [provider.name]: provider.knownModels }));
      setModelErrors(prev => ({ ...prev, [provider.name]: 'Live models unavailable. Showing the configured fallback list.' }));
    } finally {
      setLoading(null);
    }
  };

  return <div ref={rootRef} className="relative" data-testid="provider-model-picker">
    <button type="button" aria-haspopup="dialog" aria-expanded={open} aria-label={ariaLabel} onClick={() => setOpen(v => !v)} className="pa-btn w-full" style={{ '--pa-btn-fg': colors.text, '--pa-btn-bg': 'transparent', '--pa-btn-bg-hover': colors.surfaceHi, '--pa-btn-border': colors.border, '--pa-btn-pad': compact ? '6px 8px' : '8px 10px', '--pa-btn-radius': `${radius.sm}px`, justifyContent: 'space-between', fontFamily: font.mono, fontSize: textSize.micro } as CSSProperties}>
      <span className="truncate text-left">{label}</span><FiChevronDown size={12} className={open ? 'rotate-180' : ''} />
    </button>
    {open && <div role="dialog" aria-label="Choose provider and model" className="absolute left-0 top-full mt-1 z-50 w-full min-w-[280px] rounded-lg shadow-2xl overflow-hidden" style={{ backgroundColor: colors.surface, border: `1px solid ${colors.border}`, boxShadow: colors.cardShadow }}>
      <input autoFocus type="search" value={query} onChange={e => setQuery(e.target.value)} placeholder="Search providers or models…" aria-label="Search providers or models" className="w-full px-3 py-2 text-[11px] outline-none" style={{ fontFamily: font.mono, color: colors.text, backgroundColor: colors.bg, borderBottom: `1px solid ${colors.border}` }} />
      <div role="menu" className="max-h-[280px] overflow-y-auto">
        {onUseSession && <button type="button" role="menuitem" className="pa-btn w-full" onClick={() => { onUseSession(); setOpen(false); }} style={{ '--pa-btn-fg': sessionActive ? colors.cyan : colors.text, '--pa-btn-bg-hover': colors.surfaceHi, '--pa-btn-pad': '8px 10px', '--pa-btn-radius': '0', justifyContent: 'space-between', fontFamily: font.body, fontSize: textSize.micro } as CSSProperties}>Use session model {sessionActive && <FiCheck size={12} />}</button>}
        {visible.map(provider => { const allProviderModels = models[provider.name] ?? provider.knownModels; const q = query.trim().toLowerCase(); const providerModels = q && !provider.name.toLowerCase().includes(q) && !provider.displayName.toLowerCase().includes(q) ? allProviderModels.filter(m => m.toLowerCase().includes(q)) : allProviderModels; const isExpanded = expanded === provider.name; return <div key={provider.name}>
          <button type="button" aria-expanded={isExpanded} className="pa-btn w-full" onClick={() => void expand(provider)} style={{ '--pa-btn-fg': colors.text, '--pa-btn-bg-hover': colors.surfaceHi, '--pa-btn-pad': '8px 10px', '--pa-btn-radius': '0', justifyContent: 'space-between', fontFamily: font.body, fontSize: textSize.caption } as CSSProperties}><span>{provider.displayName}</span><span className="flex items-center gap-2" style={{ color: colors.textDim, fontSize: 10 }}>{loading === provider.name ? <FiLoader className="animate-spin" size={11} /> : `${providerModels.length} models`}<FiChevronDown size={11} className={isExpanded ? 'rotate-180' : ''} /></span></button>
          {isExpanded && modelErrors[provider.name] && <div role="status" className="px-3 py-2 text-[10px]" style={{ color: colors.warning, backgroundColor: colors.bg }}>{modelErrors[provider.name]}</div>}
          {isExpanded && providerModels.map(model => { const active = value.provider === provider.name && value.model === model; return <button type="button" role="menuitem" key={model} className="pa-btn w-full" onClick={() => { onChange({ provider: provider.name, model }); setOpen(false); }} style={{ '--pa-btn-fg': active ? colors.cyan : colors.text, '--pa-btn-bg-hover': colors.surfaceHi, '--pa-btn-pad': '7px 10px 7px 24px', '--pa-btn-radius': '0', justifyContent: 'space-between', fontFamily: font.mono, fontSize: textSize.micro } as CSSProperties}><span className="truncate">{model}</span>{active && <FiCheck size={12} />}</button>; })}
        </div>; })}
        {visible.length === 0 && <div className="px-3 py-4 text-center text-xs" style={{ color: colors.textMuted }}>No configured providers or matching models</div>}
      </div>
    </div>}
  </div>;
}

import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { FiCheck, FiChevronDown, FiLoader, FiSearch } from 'react-icons/fi';
import { api } from '../../lib/api';
import { useCommandCenter, type ProviderInfo } from '../../lib/store';
import { font, radius, type as typeScale } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { catalogue, describeModel, matchesQuery, mergeModelLists, type ModelEntry } from './modelCatalog';

export type ProviderModelValue = { provider: string | null; model: string | null };

type Props = {
  value: ProviderModelValue;
  onChange: (value: { provider: string; model: string }) => void;
  onUseSession?: () => void;
  sessionActive?: boolean;
  compact?: boolean;
  'aria-label'?: string;
};

/**
 * Provider-first model chooser shared by Chat, role defaults and terminals.
 *
 * Jesse: "I also cant seem to find the latest Haiku model. The dropdown picker
 * for models is ugly, it needs better design." Those turned out to be one
 * problem wearing two hats. A list of raw ids in a mono font is not a menu you
 * can find anything in — `claude-haiku-4-5-20251001` is a serial number, not a
 * name — and the list itself was lossy: the live models fetched from the
 * daemon REPLACED the provider's known-models list, and the live list is
 * filtered through the bundled canonical registry, so a model the app fully
 * supports could simply not be in the menu. See `modelCatalog.ts`.
 */
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

  /** Every model a provider can offer: live order, known list as the floor. */
  const modelsFor = useMemo(() => {
    const out: Record<string, ModelEntry[]> = {};
    for (const p of configured) {
      out[p.name] = catalogue(mergeModelLists(models[p.name] ?? [], p.knownModels));
    }
    return out;
  }, [configured, models]);

  const selectedProvider = providers.find(p => p.name === value.provider);
  const selectedLabel = value.model ? describeModel(value.model).label : null;

  const visible = useMemo(() => {
    const q = query.trim();
    if (!q) return configured;
    return configured.filter(p =>
      p.name.toLowerCase().includes(q.toLowerCase())
      || p.displayName.toLowerCase().includes(q.toLowerCase())
      || (modelsFor[p.name] ?? []).some(m => matchesQuery(m, q)));
  }, [configured, query, modelsFor]);

  // Open on the provider you are already using. Making someone expand the
  // section their own current model lives in, every single time, is how a
  // model starts feeling missing even when it is right there.
  useEffect(() => {
    if (!open) return;
    setExpanded(current => current ?? value.provider ?? (configured.length === 1 ? configured[0].name : null));
  }, [open, value.provider, configured]);

  useEffect(() => {
    const q = query.trim();
    if (!q) return;
    const match = visible[0];
    if (match) setExpanded(match.name);
  }, [query, visible]);

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

  // Fetching is driven by what is expanded, not by the click that expanded it:
  // a section opened automatically (the provider you are already using) needs
  // its live models just as much as one you opened by hand.
  useEffect(() => {
    if (!expanded || models[expanded]) return;
    let cancelled = false;
    const name = expanded;
    setLoading(name);
    void api.getProviderModels(name)
      .then(fetched => {
        if (cancelled) return;
        setModels(prev => ({ ...prev, [name]: fetched }));
        setModelErrors(prev => {
          const next = { ...prev };
          delete next[name];
          return next;
        });
      })
      .catch(() => {
        if (cancelled) return;
        // The known list is already the floor, so there is nothing to swap in
        // — only the fact that the live list is missing, which is worth saying.
        setModels(prev => ({ ...prev, [name]: [] }));
        setModelErrors(prev => ({ ...prev, [name]: 'Live models unavailable. Showing the configured fallback list.' }));
      })
      .finally(() => { if (!cancelled) setLoading(current => (current === name ? null : current)); });
    return () => { cancelled = true; };
  }, [expanded, models]);

  const expand = (provider: ProviderInfo) => {
    setExpanded(current => (current === provider.name ? null : provider.name));
  };

  const trigger = selectedLabel
    ? selectedLabel
    : (sessionActive ? 'Use session model' : 'Select a provider and model');

  return <div ref={rootRef} className="relative" data-testid="provider-model-picker">
    <button
      type="button"
      aria-haspopup="dialog"
      aria-expanded={open}
      aria-label={ariaLabel}
      onClick={() => setOpen(v => !v)}
      className="pa-btn w-full"
      style={{
        '--pa-btn-fg': colors.text,
        '--pa-btn-bg': 'transparent',
        '--pa-btn-bg-hover': colors.surfaceHi,
        '--pa-btn-border': colors.border,
        '--pa-btn-pad': compact ? '6px 10px' : '8px 12px',
        '--pa-btn-radius': `${radius.md}px`,
        justifyContent: 'space-between',
        gap: 8,
      } as CSSProperties}
    >
      <span className="truncate text-left flex items-baseline gap-2 min-w-0">
        <span className="truncate" style={{ ...typeScale.caption, fontFamily: font.body }}>{trigger}</span>
        {selectedLabel && selectedProvider && (
          <span className="truncate shrink-0" style={{ ...typeScale.micro, fontFamily: font.body, color: colors.textDim }}>
            {selectedProvider.displayName}
          </span>
        )}
      </span>
      <FiChevronDown size={13} className={open ? 'rotate-180' : ''} style={{ color: colors.textDim, transition: 'transform 160ms' }} />
    </button>

    {open && <div
      role="dialog"
      aria-label="Choose provider and model"
      className="absolute left-0 top-full mt-1.5 z-50 w-full min-w-[300px] overflow-hidden"
      style={{
        backgroundColor: colors.surface,
        border: `1px solid ${colors.border}`,
        borderRadius: radius.lg,
        boxShadow: colors.cardShadow,
      }}
    >
      <div className="flex items-center gap-2 px-3" style={{ borderBottom: `1px solid ${colors.border}` }}>
        <FiSearch size={12} style={{ color: colors.textDim, flexShrink: 0 }} />
        <input
          autoFocus
          type="search"
          value={query}
          onChange={e => setQuery(e.target.value)}
          placeholder="Search models…"
          aria-label="Search providers or models"
          className="w-full py-2.5 outline-none bg-transparent"
          style={{ ...typeScale.caption, fontFamily: font.body, color: colors.text }}
        />
      </div>

      <div role="menu" className="max-h-[320px] overflow-y-auto py-1">
        {onUseSession && <button
          type="button"
          role="menuitem"
          className="pa-btn w-full"
          onClick={() => { onUseSession(); setOpen(false); }}
          style={{
            '--pa-btn-fg': sessionActive ? colors.cyan : colors.text,
            '--pa-btn-bg-hover': colors.surfaceHi,
            '--pa-btn-pad': '8px 12px',
            '--pa-btn-radius': '0',
            justifyContent: 'space-between',
            ...typeScale.caption,
            fontFamily: font.body,
          } as CSSProperties}
        >Use session model {sessionActive && <FiCheck size={13} />}</button>}

        {visible.map(provider => {
          const all = modelsFor[provider.name] ?? [];
          const q = query.trim();
          const providerNameMatches = !!q
            && (provider.name.toLowerCase().includes(q.toLowerCase())
              || provider.displayName.toLowerCase().includes(q.toLowerCase()));
          const entries = q && !providerNameMatches ? all.filter(m => matchesQuery(m, q)) : all;
          const isExpanded = expanded === provider.name;
          return <div key={provider.name}>
            <button
              type="button"
              aria-expanded={isExpanded}
              className="pa-btn w-full"
              onClick={() => expand(provider)}
              style={{
                '--pa-btn-fg': colors.text,
                '--pa-btn-bg-hover': colors.surfaceHi,
                '--pa-btn-pad': '8px 12px',
                '--pa-btn-radius': '0',
                justifyContent: 'space-between',
                ...typeScale.caption,
                fontFamily: font.body,
              } as CSSProperties}
            >
              <span>{provider.displayName}</span>
              <span className="flex items-center gap-2" style={{ color: colors.textDim, ...typeScale.micro }}>
                {loading === provider.name
                  ? <FiLoader className="animate-spin" size={11} />
                  : `${entries.length} model${entries.length === 1 ? '' : 's'}`}
                <FiChevronDown size={11} className={isExpanded ? 'rotate-180' : ''} />
              </span>
            </button>

            {isExpanded && modelErrors[provider.name] && <div
              role="status"
              className="px-3 py-1.5"
              style={{ ...typeScale.micro, fontFamily: font.body, color: colors.warning, backgroundColor: colors.bg }}
            >{modelErrors[provider.name]}</div>}

            {isExpanded && entries.map(entry => {
              const active = value.provider === provider.name && value.model === entry.id;
              return <button
                type="button"
                role="menuitem"
                key={entry.id}
                aria-current={active || undefined}
                className="pa-btn w-full"
                onClick={() => { onChange({ provider: provider.name, model: entry.id }); setOpen(false); }}
                style={{
                  '--pa-btn-fg': active ? colors.cyan : colors.text,
                  '--pa-btn-bg-hover': colors.surfaceHi,
                  '--pa-btn-pad': '6px 12px 6px 22px',
                  '--pa-btn-radius': '0',
                  justifyContent: 'space-between',
                  gap: 10,
                  textAlign: 'left',
                } as CSSProperties}
              >
                <span className="flex flex-col min-w-0 items-start">
                  <span className="truncate w-full" style={{ ...typeScale.caption, fontFamily: font.body }}>
                    {entry.label}
                  </span>
                  {/* The id stays visible — it is what gets configured, logged
                      and quoted in a bug report — but it is secondary now,
                      under the name rather than instead of it. */}
                  {entry.label !== entry.id && <span
                    className="truncate w-full"
                    style={{ ...typeScale.micro, fontFamily: font.mono, color: colors.textDim }}
                  >{entry.id}</span>}
                </span>
                {active && <FiCheck size={13} className="shrink-0" />}
              </button>;
            })}

            {isExpanded && entries.length === 0 && loading !== provider.name && <div
              className="px-3 py-2"
              style={{ paddingLeft: 22, ...typeScale.micro, fontFamily: font.body, color: colors.textMuted }}
            >No models match.</div>}
          </div>;
        })}

        {visible.length === 0 && <div
          className="px-3 py-5 text-center"
          style={{ ...typeScale.caption, fontFamily: font.body, color: colors.textMuted }}
        >No configured providers or matching models</div>}
      </div>
    </div>}
  </div>;
}

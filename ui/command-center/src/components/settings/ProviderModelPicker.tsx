import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { FiCheck, FiChevronDown, FiLoader, FiSearch } from 'react-icons/fi';
import { api } from '../../lib/api';
import { useCommandCenter, type ProviderInfo } from '../../lib/store';
import { font, radius, space, type as typeScale } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { useGlass } from '../common/Glass';
import { catalogue, describeModel, matchesQuery, mergeModelLists, type ModelEntry } from './modelCatalog';

export type ProviderModelValue = { provider: string | null; model: string | null };

/** Below this the menu is not worth opening downward — flip it instead. */
const PANEL_MIN_HEIGHT = 220;

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
 * The user: "I also cant seem to find the latest Haiku model. The dropdown picker
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
  const glass = useGlass('glass');
  const providers = useCommandCenter(s => s.providers);
  const loadProviders = useCommandCenter(s => s.loadProviders);
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [models, setModels] = useState<Record<string, string[]>>({});
  const [modelErrors, setModelErrors] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  // Where to put the menu, in viewport coordinates.
  //
  // It used to be `position: absolute` inside the trigger's own box, which
  // works only when no ancestor clips or re-parents it. In the Build tab the
  // trigger sits in the terminal's glass tab strip — a rounded, backdrop
  // filtered bar — and the menu simply never appeared: the button toggled, and
  // the panel was painted somewhere you could not see. The user: "The select
  // provider and model dropdown doesnt open in the Build tab."
  //
  // A portal to `document.body` with fixed coordinates cannot be clipped by
  // anything, so the same menu behaves identically in the chat header, the
  // Settings rows and a terminal toolbar.
  const [anchor, setAnchor] = useState<
    { left: number; width: number; maxHeight: number; top?: number; bottom?: number } | null
  >(null);

  const placePanel = useCallback(() => {
    const el = triggerRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const width = Math.max(r.width, 300);
    // Keep it on screen: prefer left-aligned with the trigger, but never let
    // it hang off the right edge of a narrow pane.
    const left = Math.min(Math.max(8, r.left), window.innerWidth - width - 8);

    // Flip up when there is no room below, and never exceed the room there is.
    //
    // The first version of this clamped `left` and ignored `top` entirely, so
    // a trigger near the bottom of the window opened its menu past the bottom
    // of the screen. In Settings the Harness row is the lowest of the three,
    // which is exactly where the user reported he "cant see the harness models
    // available in the dropdown" — the menu was open, just off the fold.
    const GAP = 6;
    const EDGE = 8;
    const below = window.innerHeight - r.bottom - GAP - EDGE;
    const above = r.top - GAP - EDGE;
    // Only flip when it genuinely helps; a menu that jumps above the trigger
    // when both sides are cramped is more disorienting than a short one.
    const flip = below < Math.min(PANEL_MIN_HEIGHT, above) && above > below;
    setAnchor(flip
      ? { bottom: window.innerHeight - r.top + GAP, left, width, maxHeight: above }
      : { top: r.bottom + GAP, left, width, maxHeight: below });
  }, []);

  useLayoutEffect(() => {
    if (!open) return;
    placePanel();
    window.addEventListener('resize', placePanel);
    // `true` so a scroll in ANY ancestor repositions it, not just the window.
    window.addEventListener('scroll', placePanel, true);
    return () => {
      window.removeEventListener('resize', placePanel);
      window.removeEventListener('scroll', placePanel, true);
    };
  }, [open, placePanel]);

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
      const target = event.target as Node;
      // The panel is portalled, so it is NOT inside rootRef any more.
      if (rootRef.current?.contains(target) || panelRef.current?.contains(target)) return;
      setOpen(false);
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
      ref={triggerRef}
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
        gap: space.md,
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

    {open && anchor && createPortal(<div
      ref={panelRef}
      role="dialog"
      aria-label="Choose provider and model"
      className="overflow-hidden"
      // A popover is exactly what the glass material is for — the token doc
      // lists "popovers, menu chrome" as the default `glass` surface — and
      // this menu was painting a flat opaque panel instead. Reduce
      // Transparency is handled inside `useGlass`.
      style={{
        ...glass,
        position: 'fixed',
        top: anchor.top,
        bottom: anchor.bottom,
        left: anchor.left,
        width: anchor.width,
        // The panel is a column: search, then a scrolling list. Capping the
        // whole thing to the space that exists is what keeps the list's own
        // scroll meaningful instead of pushing it off the screen.
        maxHeight: anchor.maxHeight,
        display: 'flex',
        flexDirection: 'column',
        zIndex: 1000,
        border: `1px solid ${colors.border}`,
        borderRadius: radius.lg,
      }}
    >
      {/* The search field is INSET, and that is a fix rather than a flourish.
          The app's focus ring is `outline: 2px` at `outline-offset: 2px`. On a
          field that ran the full width of the panel the ring had nowhere to
          go: it drew over the panel's own rounded edge and out past it, which
          is the cyan line the user photographed cutting across the menu. Given a
          margin and its own radius, the same ring lands inside the panel. */}
      <div
        className="flex items-center gap-2"
        style={{
          margin: space.md,
          padding: `0 ${space.lg}px`,
          borderRadius: radius.md,
          background: colors.inputBg,
          border: `1px solid ${colors.border}`,
        }}
      >
        <FiSearch size={12} style={{ color: colors.textDim, flexShrink: 0 }} />
        <input
          autoFocus
          type="search"
          value={query}
          onChange={e => setQuery(e.target.value)}
          placeholder="Search models…"
          aria-label="Search providers or models"
          className="w-full bg-transparent"
          style={{
            ...typeScale.caption,
            fontFamily: font.body,
            color: colors.text,
            padding: `${space.md}px 0`,
            outline: 'none',
            border: 'none',
          }}
        />
      </div>

      <div role="menu" className="overflow-y-auto py-1" style={{ flex: 1, minHeight: 0 }}>
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
                  '--pa-btn-pad': `${space.sm}px ${space.xl}px ${space.sm}px ${space.huge}px`,
                  '--pa-btn-radius': '0',
                  justifyContent: 'space-between',
                  gap: space.lg,
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
              style={{ paddingLeft: space.huge, ...typeScale.micro, fontFamily: font.body, color: colors.textMuted }}
            >No models match.</div>}
          </div>;
        })}

        {visible.length === 0 && <div
          className="px-3 py-5 text-center"
          style={{ ...typeScale.caption, fontFamily: font.body, color: colors.textMuted }}
        >No configured providers or matching models</div>}
      </div>
    </div>, document.body)}
  </div>;
}

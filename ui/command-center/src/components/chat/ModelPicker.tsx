import { useState, useRef, useEffect, useMemo, type CSSProperties } from 'react';
import { FiChevronDown } from 'react-icons/fi';
import { useCommandCenter, type ProviderInfo } from '../../lib/store';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

export type PickerModel = {
  providerName: string;
  displayName: string;
  model: string;
};

export function filterConfiguredModels(providers: ProviderInfo[], query: string): PickerModel[] {
  const q = query.trim().toLowerCase();
  const configured = providers.filter(p => p.isConfigured);
  const out: PickerModel[] = [];
  for (const p of configured) {
    for (const model of p.knownModels) {
      if (
        !q
        || p.name.toLowerCase().includes(q)
        || p.displayName.toLowerCase().includes(q)
        || model.toLowerCase().includes(q)
      ) {
        out.push({ providerName: p.name, displayName: p.displayName, model });
      }
    }
  }
  return out;
}

export function ModelPicker() {
  const { colors } = useTheme();
  const providers = useCommandCenter(s => s.providers);
  const storeModel = useCommandCenter(s => s.currentModel);
  const loadProviders = useCommandCenter(s => s.loadProviders);
  const setDefaultProvider = useCommandCenter(s => s.setDefaultProvider);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  /** The model the daemon refused. `setDefaultProvider` used to swallow the
   *  error, so the menu closed and the label stayed on the old model with
   *  nothing on screen saying the switch had not happened. */
  const [switchError, setSwitchError] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  // Keep retrying while empty: opening chat during daemon boot left the
  // picker permanently on "no model" (the single fetch failed and nothing
  // re-triggered it). The interval dissolves as soon as providers land.
  useEffect(() => {
    if (providers.length > 0) return;
    loadProviders();
    const id = setInterval(loadProviders, 5000);
    return () => clearInterval(id);
  }, [providers.length, loadProviders]);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  useEffect(() => {
    if (!open) setQuery('');
  }, [open]);

  const defaultProvider = providers.find(p => p.isDefault);
  const displayModel = storeModel || defaultProvider?.defaultModel || 'no model';
  const matches = useMemo(() => filterConfiguredModels(providers, query), [providers, query]);

  const handleSelect = async (providerName: string, model: string) => {
    setOpen(false);
    setSwitchError(null);
    const ok = await setDefaultProvider(providerName, model);
    if (!ok) setSwitchError(model);
  };

  return (
    <div ref={ref} className="relative">
      <Button
        colors={colors}
        variant="bare"
        onClick={() => setOpen(!open)}
        style={{
          '--pa-btn-fg': colors.textMuted,
          '--pa-btn-fg-hover': colors.text,
          '--pa-btn-pad': '4px 8px',
          '--pa-btn-radius': `${radius.xs}px`,
          fontFamily: font.mono,
          fontSize: 10,
        } as CSSProperties}
      >
        {/* inline-block, not the old flex child: `Button` puts its children in
            one label span, and `max-width` does nothing to a bare inline box —
            the name would stop truncating. */}
        <span className="truncate max-w-[160px] inline-block align-middle">{displayModel}</span>
        <FiChevronDown size={10} className={`ml-1 inline-block align-middle transition ${open ? 'rotate-180' : ''}`} />
      </Button>

      {switchError && !open && (
        <div
          role="alert"
          className="absolute right-0 top-full mt-1 z-40 w-56 px-2 py-1 text-right"
          style={{ fontFamily: font.body, fontSize: 10, color: colors.danger }}
        >
          Still on {displayModel} — couldn't switch to {switchError}.
        </div>
      )}

      {open && (
        <div
          className="absolute right-0 top-full mt-1 z-50 w-72 rounded-lg shadow-2xl overflow-hidden"
          style={{ backgroundColor: colors.surface, border: `1px solid ${colors.border}`, boxShadow: colors.cardShadow }}
        >
          <input
            type="search"
            value={query}
            onChange={e => setQuery(e.target.value)}
            placeholder="Search models…"
            aria-label="Search models"
            className="w-full px-3 py-2 text-[11px] outline-none"
            style={{
              fontFamily: font.mono,
              color: colors.text,
              backgroundColor: colors.bg,
              borderBottom: `1px solid ${colors.border}`,
            }}
          />
          <div role="menu" className="max-h-[300px] overflow-y-auto">
            {matches.length === 0 && (
              <div className="px-3 py-4 text-xs text-center" style={{ color: colors.textMuted, fontFamily: font.body }}>
                {providers.filter(p => p.isConfigured).length === 0 ? 'No configured providers' : 'No matching models'}
              </div>
            )}
            {matches.map(m => {
              const isActive = m.model === displayModel;
              return (
                // `role="menuitem"` is what makes this a menu row rather than a
                // button, and `Button` would flatten it — as it would the row's
                // own layout, which pushes the model name and its provider to
                // opposite ends of the full width from one flex container that
                // `Button`'s single label span would swallow. It takes the
                // shared `.pa-btn` interaction rules instead (house pattern:
                // the card menu in ProjectsView) — this row had no hover at all
                // before, so pointing at a model looked like pointing at nothing.
                <button
                  key={`${m.providerName}-${m.model}`}
                  role="menuitem"
                  className="pa-btn w-full"
                  onClick={() => handleSelect(m.providerName, m.model)}
                  style={{
                    '--pa-btn-fg': isActive ? colors.cyan : colors.text,
                    '--pa-btn-bg-hover': colors.surfaceHi,
                    '--pa-btn-bg-active': colors.surface,
                    '--pa-btn-pad': '6px 12px',
                    '--pa-btn-radius': '0',
                    justifyContent: 'space-between',
                    textAlign: 'left',
                    fontFamily: font.mono,
                    fontSize: textSize.micro,
                  } as CSSProperties}
                >
                  <span className="truncate">{m.model}</span>
                  <span className="text-[9px] ml-2 shrink-0" style={{ color: colors.textDim }}>
                    {isActive ? 'active' : m.displayName}
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

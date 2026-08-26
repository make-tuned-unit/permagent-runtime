import { useState, useRef, useEffect, useMemo } from 'react';
import { FiChevronDown } from 'react-icons/fi';
import { useCommandCenter, type ProviderInfo } from '../../lib/store';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

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
    await setDefaultProvider(providerName, model);
  };

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1 text-[10px] transition px-2 py-1 rounded"
        style={{ fontFamily: font.mono, color: colors.textMuted }}
      >
        <span className="truncate max-w-[160px]">{displayModel}</span>
        <FiChevronDown size={10} className={`transition ${open ? 'rotate-180' : ''}`} />
      </button>

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
          <div className="max-h-[300px] overflow-y-auto">
            {matches.length === 0 && (
              <div className="px-3 py-4 text-xs text-center" style={{ color: colors.textMuted, fontFamily: font.body }}>
                {providers.filter(p => p.isConfigured).length === 0 ? 'No configured providers' : 'No matching models'}
              </div>
            )}
            {matches.map(m => {
              const isActive = m.model === displayModel;
              return (
                <button
                  key={`${m.providerName}-${m.model}`}
                  onClick={() => handleSelect(m.providerName, m.model)}
                  className="w-full text-left px-3 py-1.5 text-[11px] transition flex items-center justify-between"
                  style={{ fontFamily: font.mono, color: isActive ? colors.cyan : colors.text }}
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

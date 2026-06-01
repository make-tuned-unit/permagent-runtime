import { useState, useRef, useEffect } from 'react';
import { FiChevronDown } from 'react-icons/fi';
import { useCommandCenter } from '../../lib/store';
import { useTheme } from '../../styles/useTheme';

export function ModelPicker() {
  const { colors } = useTheme();
  const providers = useCommandCenter(s => s.providers);
  const storeModel = useCommandCenter(s => s.currentModel);
  const loadProviders = useCommandCenter(s => s.loadProviders);
  const setDefaultProvider = useCommandCenter(s => s.setDefaultProvider);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (providers.length === 0) loadProviders();
  }, [providers.length, loadProviders]);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  const defaultProvider = providers.find(p => p.isDefault);
  const configured = providers.filter(p => p.isConfigured);
  const displayModel = storeModel || defaultProvider?.defaultModel || 'no model';

  const handleSelect = async (providerName: string, model: string) => {
    setOpen(false);
    await setDefaultProvider(providerName, model);
  };

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1 text-[10px] font-mono text-dark-muted hover:text-dark-text transition px-2 py-1 rounded hover:bg-white/5"
      >
        <span className="truncate max-w-[160px]">{displayModel}</span>
        <FiChevronDown size={10} className={`transition ${open ? 'rotate-180' : ''}`} />
      </button>

      {open && (
        <div className="absolute right-0 top-full mt-1 z-50 w-64 rounded-lg border border-dark-border shadow-2xl overflow-hidden" style={{ backgroundColor: colors.surface }}>
          <div className="max-h-[300px] overflow-y-auto">
            {configured.length === 0 && (
              <div className="px-3 py-4 text-xs text-dark-muted text-center">No configured providers</div>
            )}
            {configured.map(p => (
              <div key={p.name}>
                <div className="px-3 py-1.5 text-[9px] font-mono uppercase tracking-wider text-dark-muted" style={{ backgroundColor: colors.bg }}>
                  {p.displayName}
                </div>
                {p.knownModels.map(model => {
                  const isActive = p.isDefault && model === displayModel;
                  return (
                    <button
                      key={`${p.name}-${model}`}
                      onClick={() => handleSelect(p.name, model)}
                      className={`w-full text-left px-3 py-1.5 text-[11px] font-mono hover:bg-white/5 transition flex items-center justify-between ${
                        isActive ? 'text-accent' : 'text-dark-text'
                      }`}
                    >
                      <span className="truncate">{model}</span>
                      {isActive && <span className="text-[9px] text-accent/60 ml-2 shrink-0">active</span>}
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

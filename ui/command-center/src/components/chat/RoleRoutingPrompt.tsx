import { useEffect, useState, type CSSProperties } from 'react';
import { FiZap, FiX } from 'react-icons/fi';
import { api, type PacksResponse } from '../../lib/api';
import { font, radius, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

export type RoleRoutingVariant = 'banner' | 'compact' | 'settings';

function recLines(data: PacksResponse): string[] {
  return (data.recommendation?.recommendations ?? [])
    .filter(r => r.provider && r.model)
    .map(r => `${r.role}: ${r.provider}/${r.model}`);
}

export function RoleRoutingPrompt({ variant = 'banner' }: { variant?: RoleRoutingVariant }) {
  const { colors } = useTheme();
  const [data, setData] = useState<PacksResponse | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    api.getPacks()
      .then(r => { if (active) setData(r); })
      .catch(() => { if (active) setData(null); });
    return () => { active = false; };
  }, []);

  if (dismissed || !data?.prompt) return null;

  // Resolves false on failure so the Button primitive never ticks over an
  // apply that did not happen — the message set here is the only other signal.
  const apply = async () => {
    setApplying(true);
    setError(null);
    try {
      await api.applyPacks();
      setDismissed(true);
      return true;
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not apply routing');
      return false;
    } finally {
      setApplying(false);
    }
  };

  if (variant === 'compact') {
    return (
      <span style={{ display: 'inline-flex', alignItems: 'center' }}>
        <span style={{ color: colors.textDim, margin: '0 7px' }} aria-hidden="true">·</span>
        <Button
          colors={colors}
          variant="bare"
          type="button"
          data-testid="apply-role-routing"
          className="hover:underline"
          onClick={apply}
          disabled={applying}
          style={{
            '--pa-btn-fg': colors.cyan,
            '--pa-btn-bg-hover': 'transparent',
            '--pa-btn-pad': '0',
            fontFamily: font.mono,
            fontSize: textSize.micro,
          } as CSSProperties}
        >
          {applying ? 'Applying…' : 'Apply recommended routing'}
        </Button>
      </span>
    );
  }

  const lines = recLines(data);
  const isSettings = variant === 'settings';

  return (
    <div
      data-testid="role-routing-prompt"
      className={isSettings ? undefined : 'mx-4 mb-2 rounded-lg px-4 py-3'}
      style={{
        border: `1px solid ${colors.cyan}4D`,
        backgroundColor: `${colors.cyan}0D`,
        borderRadius: isSettings ? 8 : undefined,
        padding: isSettings ? '12px 14px' : undefined,
        marginBottom: isSettings ? 16 : undefined,
      }}
    >
      <div className="flex items-start gap-3">
        <FiZap size={16} className="shrink-0 mt-0.5" style={{ color: colors.cyan }} />
        <div className="flex-1 min-w-0">
          <div className="text-[12px] mb-1" style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>
            Cheaper per-role routing is available
          </div>
          <div className="text-[11px] mb-2" style={{ fontFamily: font.body, color: colors.textMuted }}>
            Apply recommended routing so planning, edits, and mechanical work each use the
            cheapest model that can do the job. Nothing is a vendor default — this is derived
            from the models you already have.
          </div>
          {lines.length > 0 && (
            <div className="text-[10px] mb-2" style={{ fontFamily: font.mono, color: colors.textDim }}>
              {lines.join(' · ')}
            </div>
          )}
          {error && (
            <div className="text-[11px] mb-2" style={{ color: colors.danger }}>{error}</div>
          )}
          <div className="flex items-center gap-2">
            <Button
              colors={colors}
              variant="ghostOn"
              type="button"
              data-testid="apply-role-routing"
              onClick={apply}
              disabled={applying}
              style={{
                '--pa-btn-bg': `${colors.cyan}33`,
                '--pa-btn-border': 'transparent',
                '--pa-btn-bg-hover': `${colors.cyan}4D`,
                '--pa-btn-border-hover': 'transparent',
                '--pa-btn-pad': '4px 12px',
                '--pa-btn-radius': `${radius.sm}px`,
                fontFamily: font.mono,
                fontSize: textSize.micro,
              } as CSSProperties}
            >
              {applying ? 'Applying…' : 'Apply recommended routing'}
            </Button>
            <Button
              colors={colors}
              variant="bare"
              type="button"
              onClick={() => setDismissed(true)}
              style={{
                '--pa-btn-fg': colors.textMuted,
                '--pa-btn-fg-hover': colors.text,
                '--pa-btn-pad': '4px 8px',
                '--pa-btn-radius': `${radius.sm}px`,
                fontFamily: font.mono,
                fontSize: textSize.micro,
              } as CSSProperties}
            >
              <FiX size={12} className="inline mr-0.5" />
              Not now
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}

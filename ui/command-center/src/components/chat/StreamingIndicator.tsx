import { useCommandCenter } from '../../lib/store';
import { font, ease } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function StreamingIndicator() {
  const { colors, reduceMotion } = useTheme();
  const agentName = useCommandCenter(s => s.agentName);

  return (
    <div className="flex justify-start">
      <div
        className="rounded-xl px-3.5 py-2.5 relative overflow-hidden"
        style={{ backgroundColor: colors.surface, boxShadow: colors.cardShadow }}
      >
        {!reduceMotion && (
          <div
            className="absolute inset-0 opacity-[0.08]"
            style={{
              background: colors.ribbonGradient,
              animation: `shimmer 2s ${ease.inOut} infinite`,
              backgroundSize: '200% 100%',
            }}
          />
        )}
        <div className="flex items-center gap-1.5 relative">
          <span
            className="text-[11px]"
            style={{ fontFamily: font.display, fontWeight: 600, color: colors.textMuted }}
          >
            {agentName}
          </span>
          <div className="flex gap-1 ml-1">
            {reduceMotion ? (
              <span className="text-[10px]" style={{ fontFamily: font.mono, color: colors.textDim }}>...</span>
            ) : (
              <>
                <span className="h-1.5 w-1.5 rounded-full animate-bounce" style={{ backgroundColor: colors.cyan, animationDelay: '0ms' }} />
                <span className="h-1.5 w-1.5 rounded-full animate-bounce" style={{ backgroundColor: colors.cyan, animationDelay: '150ms' }} />
                <span className="h-1.5 w-1.5 rounded-full animate-bounce" style={{ backgroundColor: colors.cyan, animationDelay: '300ms' }} />
              </>
            )}
          </div>
        </div>
      </div>
      <style>{`
        @keyframes shimmer {
          0% { background-position: -200% 0; }
          100% { background-position: 200% 0; }
        }
      `}</style>
    </div>
  );
}

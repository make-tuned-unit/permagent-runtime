import { Brain } from 'lucide-react';
import { font } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';

export function BrainPanel() {
  const { colors } = useTheme();
  return (
    <div className="flex h-full w-full items-center justify-center" style={{ backgroundColor: colors.bg }}>
      <div className="text-center max-w-md">
        <Brain size={48} className="mx-auto mb-4" style={{ color: colors.cyan, opacity: 0.3 }} />
        <h2 className="text-lg" style={{ fontFamily: font.display, fontWeight: 600, color: colors.text }}>Brain</h2>
        <p className="text-sm mt-2" style={{ fontFamily: font.body, color: colors.textMuted }}>
          Memory inspector and knowledge graph live here.
        </p>
        <p className="text-xs mt-4" style={{ fontFamily: font.body, color: colors.textDim }}>
          Coming in Phase 2 Track 7
        </p>
      </div>
    </div>
  );
}

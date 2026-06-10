import { Brain } from 'lucide-react';
import { useTheme } from '../../styles/useTheme';

export function BrainPanel() {
  const { colors } = useTheme();
  return (
    <div className="flex h-full w-full items-center justify-center" style={{ backgroundColor: colors.bg }}>
      <div className="text-center max-w-md">
        <Brain size={48} className="mx-auto mb-4 text-accent/30" />
        <h2 className="text-lg font-semibold text-dark-text">Brain</h2>
        <p className="text-sm text-dark-muted mt-2">
          Memory inspector and knowledge graph live here.
        </p>
        <p className="text-xs text-dark-muted/80 mt-4">
          Coming in Phase 2 Track 7
        </p>
      </div>
    </div>
  );
}

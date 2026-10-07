import { useCommandCenter } from '../../lib/store';
import { space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { Button } from '../common/Button';

/**
 * Neutral state for the optional on-chain identity: shown when the user has not
 * linked a chitin.id slug or contract set. Nothing is fetched in this state and
 * nothing here names any particular agent or wallet.
 */
export function openPersonaSettings(): void {
  const s = useCommandCenter.getState();
  s.setPendingSettingsSection('agent');
  s.setActivePanel('settings');
}

export function IdentityUnlinked({ compact = false }: { compact?: boolean }) {
  const { colors } = useTheme();
  return (
    <div
      data-testid="identity-unlinked"
      style={{ padding: compact ? 0 : `${space.xxxl}px ${space.xl}px`, textAlign: compact ? 'left' : 'center' }}
    >
      <div style={{ fontSize: textSize.small, color: colors.textDim, lineHeight: 1.6, marginBottom: space.md }}>
        No on-chain identity linked
      </div>
      <Button colors={colors} variant="bare" flashSuccess={false} onClick={openPersonaSettings}>
        Set up in Settings, Persona
      </Button>
    </div>
  );
}

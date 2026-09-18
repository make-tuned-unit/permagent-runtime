/**
 * The placeholder for a card the layout asks for and the registry has no entry
 * for.
 *
 * The Dashboard used to answer that case with `return null`. That is fine when
 * a card type is genuinely gone, and wrong the rest of the time: two cards in
 * the *default* layout (Calendar and Council) come from the daemon-served
 * manifest, so a slow or failed manifest fetch silently removed them, leaving a
 * hole in the grid. Reset to default made it worse — it put both cards back
 * into the layout, where they rendered as nothing at all, so the button that
 * promises to restore your dashboard appeared to delete two cards from it.
 *
 * Three honest states, matching the rule the rest of the app now follows:
 * loading is not empty, a failed fetch is not an absence, and a card type that
 * really has gone says so and can be removed.
 */

import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import type { ManifestStatus } from './cards/useCardRegistry';

export function MissingCard({ type, status }: { type: string; status: ManifestStatus }) {
  const { colors } = useTheme();

  const [title, detail] = status === 'loading'
    ? ['Loading…', 'This card is served by the daemon and hasn\'t arrived yet.']
    : status === 'error'
      ? ['Card unavailable', 'The daemon didn\'t answer, so this card couldn\'t be loaded. It will come back when the connection does.']
      : ['Card no longer available', 'Nothing provides this card type any more. Remove it in Customize.'];

  return (
    <div
      data-testid={`missing-card-${type}`}
      data-status={status}
      style={{
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: space.sm,
        textAlign: 'center',
        padding: `${space.xxxl}px ${space.huge}px`,
        borderRadius: radius.lg,
        border: `1px dashed ${colors.border}`,
        background: colors.surface,
        fontFamily: font.body,
      }}
    >
      <div style={{
        fontSize: textSize.caption,
        fontWeight: 600,
        color: status === 'error' ? colors.warning : colors.textMuted,
      }}>
        {title}
      </div>
      <div style={{ fontSize: textSize.micro, color: colors.textDim, maxWidth: 280, lineHeight: 1.5 }}>
        {detail}
      </div>
      <div style={{ fontSize: 10, color: colors.textDim, fontFamily: font.mono }}>{type}</div>
    </div>
  );
}

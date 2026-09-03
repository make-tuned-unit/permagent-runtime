/**
 * AddCardPicker — pick a card to put on the dashboard.
 *
 * The chrome is `DetailModal`'s, not its own. What it had before was a
 * pixel-accurate copy of that shell — the same scrim, the same 14px/18px
 * header, the same `radius.lg` panel — and with it none of the shell's
 * keyboard floor: no focus trap, no Escape, no `role="dialog"`, and focus left
 * wherever it was when the picker opened.
 */

import { FiPlus } from 'react-icons/fi';
import { font, radius, space, textSize } from '../../styles/tokens';
import { useTheme } from '../../styles/useTheme';
import { DetailModal } from '../common/DetailModal';
import type { CardRegistryEntry } from './cards/registry';

interface Props {
  registry: Record<string, CardRegistryEntry>;
  currentCardTypes: string[];
  onSelect: (type: string) => void;
  onClose: () => void;
}

export function AddCardPicker({ registry, currentCardTypes, onSelect, onClose }: Props) {
  const { colors } = useTheme();
  const available = Object.entries(registry).filter(
    ([type]) => !currentCardTypes.includes(type)
  );

  return (
    <DetailModal
      title="Add card"
      onClose={onClose}
      width={340}
      // The rows run edge to edge and hover as full-width bands, so the body's
      // own side padding would inset them; the height keeps the panel the size
      // it has always been rather than growing to the shell's 86vh.
      bodyStyle={{ padding: '8px 0', maxHeight: 330 }}
    >
      {available.length === 0 ? (
        <div style={{
          padding: `${space.huge}px 18px`, textAlign: 'center',
          fontSize: textSize.small, color: colors.textMuted,
        }}>
          All card types are already added
        </div>
      ) : (
        available.map(([type, entry]) => (
          // Not on the Button primitive: this is the whole card the user
          // clicks to add — an icon tile beside a two-line name/description
          // block, laid out by the button itself. The primitive wraps its
          // children in one span and centres them, which would collapse
          // that layout. Left as a raw button on purpose.
          <button
            key={type}
            onClick={() => { onSelect(type); onClose(); }}
            style={{
              display: 'flex', alignItems: 'center', gap: space.xl,
              width: '100%', padding: `${space.lg}px 18px`,
              background: 'none', border: 'none',
              cursor: 'pointer', textAlign: 'left',
              transition: 'background 100ms ease',
            }}
            onMouseEnter={e => (e.currentTarget.style.background = colors.cyanSoft)}
            onMouseLeave={e => (e.currentTarget.style.background = 'none')}
          >
            <div style={{
              width: 28, height: 28, borderRadius: radius.sm,
              background: colors.cyanSoft,
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              flexShrink: 0,
            }}>
              <FiPlus size={14} style={{ color: colors.cyan }} />
            </div>
            <div style={{ minWidth: 0 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: space.sm }}>
                <span style={{ fontFamily: font.body, fontSize: textSize.small, fontWeight: 500, color: colors.text }}>
                  {entry.name}
                </span>
                {entry.source && entry.source !== 'built-in' && (
                  <span style={{
                    fontFamily: font.body, fontSize: 10, fontWeight: 600, letterSpacing: '0.04em',
                    textTransform: 'uppercase', color: colors.cyan, background: colors.cyanSoft,
                    padding: `${space.xxs}px ${space.sm}px`, borderRadius: radius.pill, flexShrink: 0,
                  }}>
                    {entry.source}
                  </span>
                )}
              </div>
              <div style={{ fontFamily: font.body, fontSize: textSize.micro, color: colors.textDim, marginTop: space.xxs }}>
                {entry.description}
              </div>
            </div>
          </button>
        ))
      )}
    </DetailModal>
  );
}

import { useEffect, useRef } from 'react';
import type { ThemeColors } from '../../../styles/tokens';

export type RailPlace = {
  id: string;
  name: string;
  subtitle: string;
};

/**
 * The places of the world, as a rail that slides in from the left.
 *
 * It replaces a cloud of chips that sat across the top of the frame and covered
 * the sky — including, by the project's own notes, about a third of the gas
 * giant. A vertical list reads in one sweep, keeps the horizon clear, and has
 * room for each place to say what it is rather than just what it is called.
 *
 * It is a rail rather than a menu: it stays where it is while you look at the
 * world, it does not steal the pointer, and the handle that opens it is always
 * reachable. Closing it is one key (Escape) or one click on the handle, and the
 * choice is the caller's to remember.
 */
export function ForumPlaceRail({
  places,
  activeId,
  open,
  colors,
  onSelect,
  onToggle,
  onHold,
}: {
  places: readonly RailPlace[];
  activeId: string;
  open: boolean;
  colors: ThemeColors;
  onSelect: (id: string) => void;
  onToggle: (open: boolean) => void;
  onHold: (held: boolean) => void;
}) {
  const listRef = useRef<HTMLDivElement | null>(null);

  // Escape closes the rail, but only when the rail itself has focus: the same
  // key exits walking, and a world you are exploring should not have its travel
  // mode cancelled by a panel you were not using.
  useEffect(() => {
    if (!open) return;
    const node = listRef.current;
    if (!node) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      if (!node.contains(document.activeElement)) return;
      event.stopPropagation();
      onToggle(false);
    };
    node.addEventListener('keydown', onKey);
    return () => node.removeEventListener('keydown', onKey);
  }, [open, onToggle]);

  return (
    <div
      className="forum-rail"
      data-open={open ? 'open' : 'closed'}
      onPointerEnter={() => onHold(true)}
      onPointerLeave={() => onHold(false)}
      onFocusCapture={() => onHold(true)}
      onBlurCapture={() => onHold(false)}
    >
      <button
        type="button"
        className="pa-btn forum-rail-handle"
        aria-expanded={open}
        aria-controls="forum-place-rail"
        onClick={() => onToggle(!open)}
        style={{ '--pa-btn-fg': colors.text } as React.CSSProperties}
      >
        <span aria-hidden="true">{open ? '‹' : '›'}</span>
        <span className="forum-rail-handle-label">Places</span>
      </button>

      <div
        className="forum-rail-list"
        id="forum-place-rail"
        ref={listRef}
        role="group"
        aria-label="World places"
        aria-hidden={!open}
      >
        {places.map((place) => {
          const active = place.id === activeId;
          return (
            <button
              key={place.id}
              type="button"
              className="pa-btn forum-rail-place"
              data-active={active ? 'yes' : 'no'}
              aria-pressed={active}
              tabIndex={open ? 0 : -1}
              onClick={() => onSelect(place.id)}
            >
              <span className="forum-rail-place-name">{place.name}</span>
              <span className="forum-rail-place-sub">{place.subtitle}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

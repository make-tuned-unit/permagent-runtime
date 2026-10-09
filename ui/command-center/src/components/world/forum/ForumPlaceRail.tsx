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
  const handleRef = useRef<HTMLButtonElement | null>(null);

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

  // The closed rail is still rendered — it is slid out with `transform` and
  // `opacity: 0`, not `display: none` — so everything in it stays in the DOM,
  // and therefore in the accessibility tree, unless we say otherwise. The
  // documented way to close it is Escape *while focus is inside it*, which is
  // exactly the case WebKit punishes: a focused descendant inside an
  // `aria-hidden` subtree makes Safari/WKWebView ignore the `aria-hidden`
  // altogether ("Blocked aria-hidden ... because its descendant retained
  // focus"). The rail then reappears to a screen reader while being invisible
  // on screen, and the reading cursor is parked on an off-screen control.
  //
  // `inert` (paired with `aria-hidden`, as WizardShell already does for its
  // inactive steps) is what actually fixes it: it drops the subtree from the
  // AX tree AND from the tab order, and blurs focus held inside it. That last
  // part would otherwise dump the user on <body>, so we hand focus back to the
  // handle — the control that reopens the rail — rather than to nowhere.
  useEffect(() => {
    if (open) return;
    const node = listRef.current;
    if (!node || !node.contains(document.activeElement)) return;
    handleRef.current?.focus();
  }, [open]);

  // `aria-hidden` alone is unreliable when focus remains inside the rail;
  // `inert` enforces the closed state in the accessibility tree.
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
        ref={handleRef}
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
        role="radiogroup"
        aria-label="World places"
        aria-hidden={!open}
        onKeyDown={(event) => {
          const offset = event.key === 'ArrowDown' || event.key === 'ArrowRight' ? 1
            : event.key === 'ArrowUp' || event.key === 'ArrowLeft' ? -1 : 0;
          if (!offset && event.key !== 'Home' && event.key !== 'End') return;
          const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="radio"]')];
          const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
          if (current < 0 || buttons.length === 0) return;
          event.preventDefault();
          const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1
            : (current + offset + buttons.length) % buttons.length;
          buttons[next].focus();
          onSelect(places[next].id);
        }}
        {...(open ? {} : { inert: '' })}
      >
        {places.map((place) => {
          const active = place.id === activeId;
          return (
            <button
              key={place.id}
              type="button"
              className="pa-btn forum-rail-place"
              data-active={active ? 'yes' : 'no'}
              // One place at a time, so these are radios rather than toggles:
              // a screen reader should say "Harbor, 14 of 14, selected", not
              // announce fourteen independent pressed/unpressed buttons.
              role="radio"
              aria-checked={active}
              tabIndex={open && active ? 0 : -1}
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

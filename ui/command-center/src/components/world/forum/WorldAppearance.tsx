import { useEffect, useId, useRef, useState } from 'react';
import { Button } from '../../common/Button';
import { radius, space, textSize } from '../../../styles/tokens';
import { useTheme } from '../../../styles/useTheme';
import { useForumActivity } from './ForumActivity';
import type { ForumLightingPreference } from './useForumLighting';

/** A one-click HUD control, independent of the Agent desk and its scroll. */
export function WorldAppearance({ lighting, onLighting, status, failed, pending, onRetry, compact, onLayout }: {
  lighting: ForumLightingPreference; onLighting: (value: ForumLightingPreference) => void;
  status: string; failed: boolean; pending: boolean; onRetry: () => void;
  compact: boolean; onLayout: () => void;
}) {
  const { colors } = useTheme();
  const active = useForumActivity();
  const [open, setOpen] = useState(false);
  const id = useId();
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const select = useRef<HTMLSelectElement>(null);
  useEffect(() => { if (!active) setOpen(false); }, [active]);
  useEffect(() => {
    if (!open) return;
    select.current?.focus({ preventScroll: true });
    const outside = (event: PointerEvent) => { if (!root.current?.contains(event.target as Node)) setOpen(false); };
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [open]);
  const close = () => { setOpen(false); trigger.current?.focus({ preventScroll: true }); };
  return <div ref={root} className="forum-appearance-control">
    <Button ref={trigger} colors={colors} variant={open ? 'ghostOn' : 'bare'} flashSuccess={false}
      aria-expanded={open} aria-controls={id} onClick={() => setOpen(value => !value)}>World appearance</Button>
    <p role="status" aria-live="polite" data-testid="forum-lighting-status" className={!pending && !failed ? 'sr-only' : undefined}
      style={{ margin: 0, fontSize: textSize.micro, maxWidth: 220 }}>{status}</p>
    <div id={id} hidden={!open} role="dialog" aria-label="World appearance" className="forum-appearance-panel"
      onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(); } }}
      style={{ padding: space.xl, borderRadius: radius.md, background: colors.surface, border: `1px solid ${colors.border}`, boxShadow: colors.elevationFloating }}>
      <label style={{ display: 'block', fontSize: textSize.small }}>World light
        <select ref={select} aria-label="World lighting" value={lighting} onChange={event => onLighting(event.target.value as ForumLightingPreference)}
          style={{ width: '100%', marginTop: space.sm }}>
          <option value="system">System appearance</option><option value="day">Warm daylight</option><option value="night">Lantern night</option>
        </select>
      </label>
      {failed && <Button colors={colors} flashSuccess={false} onClick={onRetry}>Retry lighting</Button>}
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: space.sm, marginTop: space.xl }}>
        <Button colors={colors} flashSuccess={false} onClick={onLayout}>{compact ? 'Previous campus' : 'Compact garden'}</Button>
        <Button colors={colors} flashSuccess={false} onClick={close}>Done</Button>
      </div>
    </div>
  </div>;
}

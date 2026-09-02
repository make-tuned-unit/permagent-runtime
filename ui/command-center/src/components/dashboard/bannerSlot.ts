/**
 * Home's banner slot — one at a time.
 *
 * Echo (a dormant Brain thread, resurfaced) and Learn next (the onboarding
 * coach) are unrelated features that were rendered simultaneously in
 * byte-identical shells with the same mono `✦ LABEL` kicker. Two of them
 * stacked above the grid read as one banner split in two, and neither got the
 * attention a single one would have.
 *
 * So they share a slot. Each banner reports whether it *has* something to say;
 * the slot hands it to the first one in `BANNER_ORDER` that does, and the
 * others draw nothing.
 *
 * Order is fixed rather than recency-based, because a slot that changes its
 * mind between renders would flicker. Learn next wins: a user who has not
 * tried a capability yet needs it more than a memory they can find again, and
 * it retires itself once there is nothing left to teach — at which point the
 * slot is Echo's for good.
 *
 * A banner that loses the slot must not spend its cooldown: `useBannerSlot`
 * returns the truth about what is on screen, and the caller records "shown"
 * from that, never from "I decided I could show".
 */

import { useEffect } from 'react';
import { create } from 'zustand';

export type BannerId = 'learn-next' | 'echo';

/** First one that is ready holds the slot. */
export const BANNER_ORDER: readonly BannerId[] = ['learn-next', 'echo'];

export type BannerReadiness = Partial<Record<BannerId, boolean>>;

/** Pure, so the priority rule is testable without mounting anything. */
export function slotHolder(ready: BannerReadiness): BannerId | null {
  return BANNER_ORDER.find(id => ready[id]) ?? null;
}

interface SlotState {
  ready: BannerReadiness;
  declare: (id: BannerId, ready: boolean) => void;
}

export const useBannerSlotStore = create<SlotState>((set) => ({
  ready: {},
  declare: (id, ready) => set(s => (
    s.ready[id] === ready ? s : { ready: { ...s.ready, [id]: ready } }
  )),
}));

/**
 * @param ready whether this banner has something real to show.
 * @returns whether it is the one that gets to show it.
 */
export function useBannerSlot(id: BannerId, ready: boolean): boolean {
  const declare = useBannerSlotStore(s => s.declare);
  const holder = useBannerSlotStore(s => slotHolder(s.ready));

  useEffect(() => {
    declare(id, ready);
    // Unmounting releases the slot, so navigating away from Home does not
    // leave it held by something no longer on screen.
    return () => declare(id, false);
  }, [id, ready, declare]);

  return holder === id;
}

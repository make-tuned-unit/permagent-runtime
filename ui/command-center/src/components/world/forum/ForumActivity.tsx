import { setForumStartupActivity, releaseForumStartup } from './forumStartup';
import { createContext, useContext, useLayoutEffect } from 'react';
import { useThree } from '@react-three/fiber';

/** Includes host visibility, measured size and page visibility. Portraits live
 * outside the main Canvas but must share the same rendering ownership. */
export const ForumActivity = createContext(true);
export const useForumActivity = () => useContext(ForumActivity);

export function ForumCanvasActivity({ active }: { active: boolean }) {
  const renderer = useThree(state => state.gl);
  useLayoutEffect(() => { setForumStartupActivity(renderer, active); }, [renderer, active]);
  useLayoutEffect(() => () => releaseForumStartup(renderer), [renderer]);
  const setEvents = useThree(state => state.setEvents);
  const controls = useThree(state => state.controls) as { enabled?: boolean } | null;
  useLayoutEffect(() => { setEvents({ enabled: active }); }, [active, setEvents]);
  useLayoutEffect(() => {
    if (active || !controls) return;
    const previous = controls.enabled;
    controls.enabled = false;
    return () => { controls.enabled = previous; };
  }, [active, controls]);
  return null;
}

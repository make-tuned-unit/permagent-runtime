import { useEffect, useState } from 'react';
import { scheduleForumStartup } from './forumStartup';
/** Canvas, core scene, actors, then postprocessing get separate paint turns.
 * Retain completed phases on hide; cancel only the pending turn. */
export function useForumStartupStage(active: boolean, count: number) {
  const [stage, setStage] = useState(0);
  useEffect(() => {
    if (!active || stage >= count) return;
    return scheduleForumStartup(`ForumView stage ${stage + 1}`, () => { setStage(stage + 1); return true; });
  }, [active, count, stage]);
  return stage;
}

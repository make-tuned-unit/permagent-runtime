import { useEffect, useMemo, useState, type DependencyList } from 'react';
import { useForumActivity } from './ForumActivity';
import { advanceForumBuild, scheduleForumStartup } from './forumStartup';

/** Keep a partially built scene across workspace hides; release it on unmount.
 * Factories are generators: creating one here performs no geometry work. */
export function useForumBuild<T>(name: string, factory: () => Generator<void, T>, deps: DependencyList, dispose: (value: T) => void) {
  const active = useForumActivity();
  const [, update] = useState(0);
  const job = useMemo(() => ({ steps: null as Generator<void, T> | null, factory, value: null as T | null, error: null as unknown }), deps);
  useEffect(() => {
    job.steps = job.factory();
    return () => {
      job.steps?.return(undefined as never);
      if (job.value) dispose(job.value);
      job.steps = null; job.value = null; job.error = null;
    };
  }, [job]);
  useEffect(() => {
    if (!active || job.value || job.error) return;
    return scheduleForumStartup(name, () => {
      try { return advanceForumBuild(job.steps!, value => { job.value = value; update(n => n + 1); }); }
      catch (error) { job.error = error; update(n => n + 1); return true; }
    });
  }, [active, job, name]);
  if (job.error) throw job.error;
  return job.value;
}

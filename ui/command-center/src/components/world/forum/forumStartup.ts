/** One bounded startup slice per browser task, shared by every World builder.
 * rAF + a task (not a resolved Promise) lets the loading HUD paint and input run.
 * Work is queued globally so independent builders cannot consume one frame each.
 */
type Task = { owner?: object; name: string; run: () => boolean; cancelled: boolean };
const queue: Task[] = [];
const activity = new WeakMap<object, boolean>();
const runnable = (task: Task) => !task.cancelled && (!task.owner || activity.get(task.owner) !== false);
let frame: number | undefined;
let timer: ReturnType<typeof setTimeout> | undefined;
export type ForumStartupSample = { step: string; start: number; ms: number };
let observer: ((sample: ForumStartupSample) => void) | undefined;
/** Opt-in diagnostics; no retained samples or performance entries in production. */
export function observeForumStartup(next?: typeof observer) { observer = next; }
export function measureForumStartup<T>(step: string, run: () => T): T {
  const start = performance.now();
  try { return run(); } finally { observer?.({ step, start, ms: performance.now() - start }); }
}
function schedule() {
  if (frame !== undefined || timer !== undefined || !queue.some(runnable)) return;
  frame = requestAnimationFrame(() => {
    frame = undefined;
    timer = setTimeout(() => {
      timer = undefined;
      const index = queue.findIndex(runnable);
      const task = index < 0 ? undefined : queue.splice(index, 1)[0];
      try {
        if (task) {
          const done = measureForumStartup(task.name, task.run);
          if (!done && !task.cancelled) queue.push(task);
        }
      } finally { schedule(); }
    }, 0);
  });
}
export function scheduleForumStartup(name: string, run: () => boolean, owner?: object) {
  const task: Task = { name, run, owner, cancelled: false }; queue.push(task); schedule();
  return () => {
    task.cancelled = true;
    const index = queue.indexOf(task); if (index >= 0) queue.splice(index, 1);
    if (!queue.some(runnable)) {
      if (frame !== undefined) cancelAnimationFrame(frame);
      if (timer !== undefined) clearTimeout(timer);
      frame = undefined; timer = undefined;
    }
  };
}
export function advanceForumBuild<T>(steps: Generator<void, T>, complete: (value: T) => void) {
  const start = performance.now();
  // Time budget plus an iteration cap also bounds work on coarse clocks.
  for (let n = 0; n < 512; n++) {
    const result = steps.next();
    if (result.done) { complete(result.value); return true; }
    if (performance.now() - start >= 8) break;
  }
  return false;
}

/** Dynamic import's network/evaluation promise is not a synchronous CPU span. */
export function loadForumModule<T>(name: string, load: () => Promise<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    scheduleForumStartup(`${name} dispatch`, () => { try { void load().then(resolve, reject); } catch(error) { reject(error); } return true; });
  });
}

/** GPU uploads may be waiting in Suspense, before their component effects run.
 * Give those tasks the Canvas' explicit activity instead of inferring size. */
export function setForumStartupActivity(owner: object, active: boolean) {
  activity.set(owner, active);
  if (!queue.some(runnable)) {
    if (frame !== undefined) cancelAnimationFrame(frame);
    if (timer !== undefined) clearTimeout(timer);
    frame = undefined; timer = undefined;
  }
  schedule();
}
export function releaseForumStartup(owner: object) {
  setForumStartupActivity(owner,false);
  // React StrictMode immediately replays the Canvas effects. Let that replay
  // reclaim its pending uploads rather than orphaning their Suspense promises.
  queueMicrotask(() => {
    if(activity.get(owner)!==false)return;
    for(let i=queue.length-1;i>=0;i--)if(queue[i].owner===owner)queue.splice(i,1);
    schedule();
  });
}

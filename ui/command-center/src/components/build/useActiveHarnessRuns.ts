import { useEffect, useState } from 'react';
import { api, type HarnessRunView } from '../../lib/api';

const POLL_MS = 5000;

/** Live coding-harness runs, polled. A failed read is an empty list: Build then
 *  shows the plain idle state rather than guessing a run exists. */
export function useActiveHarnessRuns(): HarnessRunView[] {
  const [runs, setRuns] = useState<HarnessRunView[]>([]);
  useEffect(() => {
    let alive = true;
    const read = () => {
      api.getActiveHarnessRuns()
        .then(list => { if (alive) setRuns(Array.isArray(list) ? list : []); })
        .catch(() => { if (alive) setRuns([]); });
    };
    read();
    const timer = setInterval(read, POLL_MS);
    return () => { alive = false; clearInterval(timer); };
  }, []);
  return runs;
}

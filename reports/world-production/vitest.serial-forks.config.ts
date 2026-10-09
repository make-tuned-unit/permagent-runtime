// Review-runner override only; application and test assertions are unchanged.
// Node 24's concurrent Sparkplug compiler crashed in the 2026-10-09 broad run.
// Keep each test file isolated and apply the runtime workaround to every fork.
import base from '../../ui/command-center/vite.config';

export default {
  ...base,
  test: {
    ...base.test,
    pool: 'forks',
    poolOptions: {
      forks: {
        minForks: 1,
        maxForks: 1,
        execArgv: ['--no-concurrent-sparkplug'],
      },
    },
  },
};

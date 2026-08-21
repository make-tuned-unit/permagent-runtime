import { describe, it, expect } from 'vitest';
import { trustEnvOverrideNotice } from './autonomy';

// Re-enable-gate epic part B: with GOOSE_MODE set in the daemon's environment,
// the Autonomy panel used to highlight the YAML mode ("Automatic"), show no
// warning, and silently write YAML that changed nothing. These pin the
// warning logic fed by the new env-aware `effective_goose_mode` field.
describe('trustEnvOverrideNotice', () => {
  it('warns when the env-resolved mode diverges from the YAML selection', () => {
    const notice = trustEnvOverrideNotice('approve', 'auto');
    expect(notice).toBe(
      "The daemon's environment overrides this to 'approve' — clear the autonomy-mode override in the environment to control it here.",
    );
  });

  it('names the effective mode, whatever it is', () => {
    expect(trustEnvOverrideNotice('smart_approve', 'chat')).toContain("'smart_approve'");
    expect(trustEnvOverrideNotice('chat', 'auto')).toContain("'chat'");
  });

  it('stays quiet when effective and selected agree (env override is a no-op)', () => {
    expect(trustEnvOverrideNotice('approve', 'approve')).toBeNull();
    expect(trustEnvOverrideNotice('auto', 'auto')).toBeNull();
  });

  it('stays quiet on daemons that predate effective_goose_mode', () => {
    expect(trustEnvOverrideNotice(undefined, 'auto')).toBeNull();
    expect(trustEnvOverrideNotice(null, 'auto')).toBeNull();
    expect(trustEnvOverrideNotice('', 'auto')).toBeNull();
  });

  it('stays quiet while the panel is still loading the selection', () => {
    expect(trustEnvOverrideNotice('approve', null)).toBeNull();
  });
});

import { describe, expect, it } from 'vitest';
import { safeWizardError } from './wizardErrors';

describe('safeWizardError', () => {
  it('redacts credentials and local paths while preserving the failure category', () => {
    const safe = safeWizardError(
      new Error('provider check failed: api_key=sk-live-sentinel /Users/alice/private/project'),
      'operation failed',
    );
    expect(safe).toContain('provider check failed');
    expect(safe).not.toContain('sk-live-sentinel');
    expect(safe).not.toContain('/Users/alice');
    expect(safe).toContain('[credential redacted]');
    expect(safe).toContain('[local path]');
  });
});

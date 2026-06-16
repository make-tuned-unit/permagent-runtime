import { describe, it, expect, vi, beforeEach } from 'vitest';

// Isolate the daemon fetch — the real ./api module has load-time side effects.
vi.mock('./api', () => ({ apiFetch: vi.fn() }));

import { apiFetch } from './api';
import {
  classifySkew,
  versionBannerState,
  fetchDaemonVersion,
  detectVersionSkew,
  type DaemonVersionInfo,
} from './version';

const mockApiFetch = apiFetch as unknown as ReturnType<typeof vi.fn>;

const daemonInfo = (version: string): DaemonVersionInfo => ({
  git_sha: 'abc123def',
  git_branch: 'main',
  git_dirty: 'false',
  build_timestamp: '2026-06-15T00:00:00Z',
  rust_version: '1.85.0',
  spectral_pin: '2c1f6bf',
  uptime_seconds: 42,
  permagentd_version: version,
});

beforeEach(() => mockApiFetch.mockReset());

describe('classifySkew — induced app↔daemon mismatches', () => {
  it('major version difference → major', () => {
    expect(classifySkew('1.31.0', '2.0.0').level).toBe('major');
  });
  it('minor version difference → minor', () => {
    expect(classifySkew('1.31.0', '1.30.5').level).toBe('minor');
  });
  it('patch-only difference → patch (benign)', () => {
    expect(classifySkew('1.31.0', '1.31.4').level).toBe('patch');
  });
  it('identical → match', () => {
    expect(classifySkew('1.31.0', '1.31.0').level).toBe('match');
  });
  it('missing daemon version → unknown (no false alarm)', () => {
    expect(classifySkew('1.31.0', null).level).toBe('unknown');
  });
  it('unparseable version → unknown', () => {
    expect(classifySkew('garbage', '1.0.0').level).toBe('unknown');
  });
  it('tolerates v-prefix and pre-release/build metadata', () => {
    expect(classifySkew('v1.31.0', '1.31.0-rc1+abc').level).toBe('match');
  });
});

describe('versionBannerState — what the banner surfaces', () => {
  it('major skew renders a red incompatibility warning naming both versions', () => {
    const state = versionBannerState(classifySkew('1.31.0', '2.0.0'));
    expect(state?.severity).toBe('major');
    expect(state?.message).toContain('app v1.31.0');
    expect(state?.message).toContain('daemon v2.0.0');
    expect(state?.message).toContain('incompatible');
  });
  it('minor skew renders an amber advisory', () => {
    const state = versionBannerState(classifySkew('1.31.0', '1.30.0'));
    expect(state?.severity).toBe('minor');
    expect(state?.message).toContain('update when convenient');
  });
  it('match / patch / unknown / null render nothing', () => {
    expect(versionBannerState(classifySkew('1.31.0', '1.31.0'))).toBeNull();
    expect(versionBannerState(classifySkew('1.31.0', '1.31.9'))).toBeNull();
    expect(versionBannerState(classifySkew('1.31.0', null))).toBeNull();
    expect(versionBannerState(null)).toBeNull();
  });
});

describe('daemon /api/version fetch path', () => {
  it('fetchDaemonVersion hits the public /api/version endpoint', async () => {
    mockApiFetch.mockResolvedValue(daemonInfo('1.31.0'));
    const info = await fetchDaemonVersion();
    expect(mockApiFetch).toHaveBeenCalledWith('/api/version');
    expect(info.permagentd_version).toBe('1.31.0');
  });

  it('detectVersionSkew queries /api/version and resolves gracefully', async () => {
    // No Tauri shell in the node test env → app version is null, so the skew is
    // 'unknown' even on a successful fetch. This exercises the full fetch path
    // and the graceful-resolution contract. The daemon-unreachable case (daemon
    // version null) is covered by classifySkew(x, null) → 'unknown' above, and
    // detectVersionSkew wraps fetchDaemonVersion in try/catch so an outage maps
    // to that same null daemon version.
    mockApiFetch.mockResolvedValue(daemonInfo('1.31.0'));
    const skew = await detectVersionSkew();
    expect(mockApiFetch).toHaveBeenCalledWith('/api/version');
    expect(skew.level).toBe('unknown');
  });
});

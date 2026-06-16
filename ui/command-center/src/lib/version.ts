import { apiFetch } from './api';

// Build metadata returned by the daemon's public /api/version endpoint.
// Mirrors VersionInfo in crates/goose-server/src/routes/version.rs.
export interface DaemonVersionInfo {
  git_sha: string;
  git_branch: string;
  git_dirty: string;
  build_timestamp: string;
  rust_version: string;
  spectral_pin: string;
  uptime_seconds: number;
  permagentd_version: string;
}

export type SkewLevel = 'match' | 'patch' | 'minor' | 'major' | 'unknown';

export interface VersionSkew {
  level: SkewLevel;
  appVersion: string | null;
  daemonVersion: string | null;
}

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** Daemon build metadata (public endpoint — no auth required, but apiFetch adds it harmlessly). */
export function fetchDaemonVersion(): Promise<DaemonVersionInfo> {
  return apiFetch<DaemonVersionInfo>('/api/version');
}

/**
 * The desktop app's own version (from tauri.conf.json). Only available inside
 * the Tauri shell; returns null in a plain browser (dev) where no app exists.
 */
export async function getAppVersion(): Promise<string | null> {
  if (!isTauri) return null;
  try {
    const { getVersion } = await import('@tauri-apps/api/app');
    return await getVersion();
  } catch {
    return null;
  }
}

type Semver = { major: number; minor: number; patch: number };

function parseSemver(v: string | null | undefined): Semver | null {
  if (!v) return null;
  // Strip leading 'v' and any pre-release/build metadata (-alpha, +sha).
  const core = v.trim().replace(/^v/, '').split(/[-+]/, 1)[0];
  const parts = core.split('.');
  if (parts.length < 2) return null;
  const [major, minor, patch] = [parts[0], parts[1], parts[2] ?? '0'].map(n => Number(n));
  if ([major, minor, patch].some(n => !Number.isFinite(n))) return null;
  return { major, minor, patch };
}

/**
 * Classify the relationship between the app and daemon versions. Unparseable or
 * missing versions return 'unknown' (no false-alarm banner). Patch-only drift is
 * benign; minor/major drift is surfaced to the user.
 */
export function classifySkew(appVersion: string | null, daemonVersion: string | null): VersionSkew {
  const a = parseSemver(appVersion);
  const d = parseSemver(daemonVersion);
  if (!a || !d) return { level: 'unknown', appVersion, daemonVersion };
  let level: SkewLevel;
  if (a.major !== d.major) level = 'major';
  else if (a.minor !== d.minor) level = 'minor';
  else if (a.patch !== d.patch) level = 'patch';
  else level = 'match';
  return { level, appVersion, daemonVersion };
}

export interface BannerState {
  severity: 'minor' | 'major';
  message: string;
}

/**
 * Pure presentation decision for the skew banner. Returns null when nothing
 * should be surfaced (match/patch/unknown), else the severity + message. Kept
 * separate from the component so the surfacing logic is unit-testable without a
 * DOM.
 */
export function versionBannerState(skew: VersionSkew | null): BannerState | null {
  if (!skew || (skew.level !== 'minor' && skew.level !== 'major')) return null;
  const base = `app v${skew.appVersion} ↔ daemon v${skew.daemonVersion}`;
  if (skew.level === 'major') {
    return { severity: 'major', message: `Version mismatch: ${base} — these builds may be incompatible. Restart or update Permagent to realign.` };
  }
  return { severity: 'minor', message: `Version skew: ${base} — a minor version difference; update when convenient.` };
}

/**
 * Fetch both versions and classify. Never throws: each source is resolved
 * independently, so a daemon outage still leaves the app version known (and
 * yields 'unknown' skew rather than crashing the banner).
 */
export async function detectVersionSkew(): Promise<VersionSkew> {
  const appVersion = await getAppVersion().catch(() => null);
  let daemonVersion: string | null = null;
  try {
    daemonVersion = (await fetchDaemonVersion()).permagentd_version;
  } catch {
    daemonVersion = null;
  }
  return classifySkew(appVersion, daemonVersion);
}

import { useEffect, useRef, useState } from 'react';
import { radius } from '../../../styles/tokens';
import { portraitSpec } from './portrait';

/** The roster renders ordinary images: a single, lazy offscreen renderer creates
 * each World character's still only when a portrait enters the viewport. */
export function AgentPortrait({ agentId, size = 56 }: { agentId: string; size?: number }) {
  const spec = portraitSpec(agentId);
  const host = useRef<HTMLSpanElement>(null);
  const [snapshot, setSnapshot] = useState<{ worldId: string; url: string } | null>(null);
  const source = snapshot?.worldId === spec.worldId ? snapshot.url : null;

  useEffect(() => {
    setSnapshot(null);
    if (!spec.worldId || typeof window === 'undefined' || typeof WebGLRenderingContext === 'undefined') return;
    let active = true;
    let requested = false;
    const request = () => {
      if (requested) return;
      requested = true;
      // This import keeps Three and the GLB loader out of Settings until a
      // mapped portrait is actually visible. The module shares requests across
      // roster rows, detail headers, and React StrictMode remounts.
      void import('./thumbnailCache')
        .then(({ getAgentThumbnail }) => getAgentThumbnail(spec.worldId!))
        .then(url => { if (active && url) setSnapshot({ worldId: spec.worldId!, url }); })
        .catch(() => { if (active) setSnapshot(null); });
    };
    if (typeof IntersectionObserver === 'undefined') {
      request();
    } else {
      const observer = new IntersectionObserver(entries => {
        if (entries.some(entry => entry.isIntersecting)) {
          observer.disconnect();
          request();
        }
      });
      if (host.current) observer.observe(host.current);
      return () => { active = false; observer.disconnect(); };
    }
    return () => { active = false; };
  }, [spec.worldId]);

  return <span
    ref={host}
    role="img"
    aria-label={source ? `${spec.label} — World portrait` : spec.worldId ? `${spec.label} — portrait unavailable` : `${spec.label} — no World portrait`}
    data-testid={`agent-portrait-${agentId}`}
    data-world-id={spec.worldId ?? ''}
    data-trim-color={spec.trimColor ?? ''}
    style={{ width: size, height: size, flexShrink: 0, display: 'inline-grid', placeItems: 'center', overflow: 'hidden', borderRadius: radius.md, border: '1px solid', borderColor: spec.trimColor ?? 'currentColor', background: 'transparent' }}
  >
    {source
      ? <img src={source} alt="" width={size} height={size} style={{ display: 'block', width: '100%', height: '100%', objectFit: 'cover' }} />
      : <span aria-hidden="true" style={{ color: spec.trimColor ?? 'currentColor', fontSize: size < 40 ? 15 : 24, lineHeight: 1 }}>◇</span>}
  </span>;
}

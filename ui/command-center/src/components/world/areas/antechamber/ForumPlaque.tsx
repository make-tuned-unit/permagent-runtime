// ForumPlaque — the honest sign at the Mesh gate (#306 preparation).
//
// Reads ONLY shared/meshStatus.ts (the frozen Phase-0 contract): dormant copy
// while the Mesh is offline, live peer count when it ships — the world's
// gateway to the Forum where sovereign chiti-identified agents commingle.
// When Mesh lands, real state plugs into meshStatus and this sign updates
// without touching area code. No fake peers, ever (atmosphere honesty law).

import { Html } from '@react-three/drei';
import { useMeshStatus } from '../../shared/meshStatus';
import { ENV } from '../../shared/palette';
import { radius, space } from '../../../../styles/tokens';

export function ForumPlaque() {
  const mesh = useMeshStatus();
  const online = mesh.state === 'connected';
  const accent = online ? ENV.neonCyan : '#5A6478';

  return (
    <Html position={[0, 0.55, 2.6]} center distanceFactor={16} style={{ pointerEvents: 'none' }}>
      <div
        style={{
          padding: `${space.xs}px ${space.xl}px`,
          borderRadius: radius.xs,
          background: 'rgba(10, 14, 26, 0.85)',
          border: `1px solid ${accent}66`,
          boxShadow: online ? `0 0 14px ${accent}44` : 'none',
          color: '#E8E4DD',
          fontFamily: 'JetBrains Mono, monospace',
          fontSize: 10,
          letterSpacing: '0.14em',
          textAlign: 'center',
          whiteSpace: 'nowrap',
        }}
      >
        <div style={{ color: accent, fontWeight: 700 }}>THE FORUM</div>
        <div style={{ marginTop: space.xxs, opacity: 0.75 }}>
          {mesh.state === 'connected'
            ? `MESH ONLINE — ${mesh.peerCount} SOVEREIGN PEER${mesh.peerCount === 1 ? '' : 'S'}`
            : mesh.state === 'connecting'
              ? 'MESH CONNECTING…'
              : 'OPENS WHEN THE MESH DOES'}
        </div>
      </div>
    </Html>
  );
}

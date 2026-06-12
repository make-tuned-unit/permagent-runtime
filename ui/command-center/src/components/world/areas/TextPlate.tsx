// TextPlate — engraved-label text on a plane via CanvasTexture.
// No network font loading (packaged WKWebView must work offline), one draw
// call per plate. Light tier: text only, engraved into stone/metal surfaces.

import { useMemo, useEffect } from 'react';
import * as THREE from 'three';

interface TextPlateProps {
  text: string;
  /** World height of the rendered plate. Width derives from text aspect. */
  height?: number;
  color?: string;
  /** 0–1 emissive-feel opacity. */
  opacity?: number;
  position?: [number, number, number];
  rotation?: [number, number, number];
}

const FONT = '600 64px "SF Mono", ui-monospace, Menlo, monospace';
const PAD = 24;

export function TextPlate({
  text,
  height = 0.6,
  color = '#FFFFFF',
  opacity = 0.9,
  position,
  rotation,
}: TextPlateProps) {
  const { texture, aspect } = useMemo(() => {
    const canvas = document.createElement('canvas');
    const ctx = canvas.getContext('2d')!;
    ctx.font = FONT;
    const w = Math.ceil(ctx.measureText(text).width) + PAD * 2;
    const h = 96 + PAD;
    canvas.width = w;
    canvas.height = h;
    const ctx2 = canvas.getContext('2d')!;
    ctx2.font = FONT;
    ctx2.fillStyle = color;
    ctx2.textBaseline = 'middle';
    ctx2.textAlign = 'center';
    ctx2.fillText(text, w / 2, h / 2);
    const tex = new THREE.CanvasTexture(canvas);
    tex.anisotropy = 4;
    tex.colorSpace = THREE.SRGBColorSpace;
    return { texture: tex, aspect: w / h };
  }, [text, color]);

  useEffect(() => () => texture.dispose(), [texture]);

  return (
    <mesh position={position} rotation={rotation}>
      <planeGeometry args={[height * aspect, height]} />
      <meshBasicMaterial
        map={texture}
        transparent
        opacity={opacity}
        depthWrite={false}
        side={THREE.DoubleSide}
        toneMapped={false}
      />
    </mesh>
  );
}

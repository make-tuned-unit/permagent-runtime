// Procedural stone canvas texture (#16 realism pass) — speckle noise, faint
// veining, optional vertical fluting bands (carved-groove shading when wrapped
// around a cylinder). One small canvas per (base, flutes) variant, cached.
// Shared by the hall structure and the legacy prop materials so every marble
// surface reads as stone instead of flat blockout-gray.

import * as THREE from 'three';

const cache = new Map<string, THREE.CanvasTexture>();

export function makeStoneTexture(base: string, flutes = 0): THREE.CanvasTexture {
  const key = `${base}:${flutes}`;
  const cached = cache.get(key);
  if (cached) return cached;
  const size = 256;
  const c = document.createElement('canvas');
  c.width = c.height = size;
  const ctx = c.getContext('2d')!;
  ctx.fillStyle = base;
  ctx.fillRect(0, 0, size, size);
  const img = ctx.getImageData(0, 0, size, size);
  for (let i = 0; i < img.data.length; i += 4) {
    const n = (Math.random() - 0.5) * 14;
    img.data[i] += n; img.data[i + 1] += n; img.data[i + 2] += n;
  }
  ctx.putImageData(img, 0, 0);
  // Faint diagonal veining
  ctx.globalAlpha = 0.05;
  ctx.strokeStyle = '#ffffff';
  for (let i = 0; i < 10; i++) {
    const x = Math.random() * size, y = Math.random() * size;
    ctx.beginPath();
    ctx.moveTo(x, y);
    ctx.bezierCurveTo(x + 40, y + 18, x + 60, y - 22, x + 110, y + 8);
    ctx.lineWidth = 1 + Math.random();
    ctx.stroke();
  }
  ctx.globalAlpha = 1;
  // Vertical fluting: soft dark groove + bright edge per band.
  if (flutes > 0) {
    const w = size / flutes;
    for (let f = 0; f < flutes; f++) {
      const x0 = f * w;
      const g = ctx.createLinearGradient(x0, 0, x0 + w, 0);
      g.addColorStop(0, 'rgba(0,0,0,0)');
      g.addColorStop(0.42, 'rgba(0,0,0,0.16)');
      g.addColorStop(0.5, 'rgba(0,0,0,0.22)');
      g.addColorStop(0.58, 'rgba(0,0,0,0.16)');
      g.addColorStop(0.78, 'rgba(255,255,255,0.06)');
      g.addColorStop(1, 'rgba(0,0,0,0)');
      ctx.fillStyle = g;
      ctx.fillRect(x0, 0, w, size);
    }
  }
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  tex.wrapS = tex.wrapT = THREE.RepeatWrapping;
  cache.set(key, tex);
  return tex;
}

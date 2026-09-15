const fract = (x: number) => x - Math.floor(x);
export const skyHash = (x: number) => fract(Math.sin(x * 127.1 + 311.7) * 43758.5453);
/** One brief meteor in a varying window; independent of frame rate. */
export function meteorAt(seconds: number, reduceMotion: boolean) {
  if (reduceMotion || seconds < 0) return null;
  const slot = Math.floor(seconds / 45);
  const start = 6 + skyHash(slot + 4) * 28;
  const age = seconds - slot * 45 - start;
  if (age < 0 || age > 1.4) return null;
  return { progress: age / 1.4, azimuth: skyHash(slot + 31) * Math.PI * 2, height: 85 + skyHash(slot + 13) * 55, opacity: Math.sin(age / 1.4 * Math.PI) };
}
export function birdFlight(seconds: number, index: number) {
  const t = seconds * .075 + index * 1.73;
  const radius = 65 + index * 9;
  const bank = .1 * Math.sin(t * .7);
  // Long glides punctuated by small, unsynchronized flapping bouts.
  const flap = Math.max(0, Math.sin(seconds * .43 + index * 2.1)) * Math.sin(seconds * 4.1 + index) * .3;
  return { x: Math.cos(t) * radius, y: 30 + index * 3 + Math.sin(t * 1.3) * 3, z: Math.sin(t) * radius, yaw: -t, bank, flap };
}

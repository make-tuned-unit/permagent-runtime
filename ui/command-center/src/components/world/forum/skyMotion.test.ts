import { expect, it } from 'vitest';
import { birdFlight, meteorAt } from './skyMotion';
it('keeps shooting stars sparse and suppresses them with reduced motion',()=>{
  let visible=0;const headings=new Set<number>();
  for(let t=0;t<600;t+=.1){const shot=meteorAt(t,false);if(shot){visible++;headings.add(shot.azimuth);expect(shot.opacity).toBeGreaterThanOrEqual(0);expect(shot.opacity).toBeLessThanOrEqual(1);}expect(meteorAt(t,true)).toBeNull();}
  expect(visible).toBeGreaterThan(100);expect(visible).toBeLessThan(220);expect(headings.size).toBeGreaterThan(10);
});
it('keeps the flock above the tallest architecture, on smooth finite paths',()=>{
  for(let i=0;i<6;i++)for(let t=0;t<180;t+=.2){const a=birdFlight(t,i),b=birdFlight(t+.016,i);expect(a.y).toBeGreaterThan(20);expect(Math.hypot(b.x-a.x,b.z-a.z)).toBeLessThan(.2);expect(Math.abs(a.flap)).toBeLessThanOrEqual(.3);}
});

import { expect, it } from 'vitest';
import { speechOpening, smoothSpeech } from './agentSpeech';
it('opens only for real playback energy, closes during silence, listening and processing',()=>{
  const speech=new Uint8Array(32).fill(100);
  expect(speechOpening('playing',speech)).toBeGreaterThan(.7);
  for(const state of ['ready','recording','processing','idle','error'])expect(speechOpening(state,speech)).toBe(0);
  expect(speechOpening('playing',null)).toBe(0);
  expect(speechOpening('playing',new Uint8Array(32))).toBe(0);
  expect(speechOpening('playing',new Uint8Array(32).fill(255))).toBe(1);
  let mouth=1;for(let i=0;i<30;i++)mouth=smoothSpeech(mouth,0,1/60);expect(mouth).toBeLessThan(.001);
});

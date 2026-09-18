import { ForumSky } from './ForumSky';
import { ENV } from '../shared/palette';

// Day is a sunrise: a warm key held low (~15 degrees) on a north-westerly
// bearing, so the commons overlook and the west face of everything catches it,
// with a cooler sky fill opposite.
//
// Night is moonlight. It used to be a violet hemisphere sky over an *amber*
// directional fill, and the two mixed on the pale paving into the lavender-pink
// commons floor of `assets/world/forum/browser/v3-world-night.webp` — a warm
// cast on every lit surface, where the reference
// (`docs/design/solar-forum/north-star.png`) is a deep blue night whose only
// warmth comes from the emissives themselves. Both broad lights are cool now
// and the ground bounce is a deep blue rather than a violet grey. The warmth
// is where it belongs: the nine amber point lights below are the lanterns, the
// braziers and the strand lights, and they are untouched. So are the
// emissives, which take no light at all — `emissiveIntensity` is independent
// of every light in this file, so nothing here can dim them.
export const FORUM_LIGHT = {
  day: { background: ENV.marble, sky: '#FFE9CC', ground: ENV.marbleVein, ambient: 1.2, key: '#FFD9A8', strength: 2.6, fill: '#9FC0E8', fillStrength: .55 },
  night: { background: ENV.deepVoid, sky: '#7FA0DE', ground: '#16233C', ambient: .48, key: '#A8C4FF', strength: .85, fill: '#5C7FC6', fillStrength: .34 },
} as const;
/** Sun bearing, matching the sunrise sun drawn in ForumSky. asin(.274) ~= 15.9 deg. */
const DAY_KEY_POSITION: [number,number,number] = [-104,30,-18];
const NIGHT_KEY_POSITION: [number,number,number] = [-15,28,12];
const DAY_FILL_POSITION: [number,number,number] = [46,20,28];
const NIGHT_FILL_POSITION: [number,number,number] = [12,8,-8];
export function ForumLighting({ appearance }: { appearance: 'day' | 'night' }) {
  const light = FORUM_LIGHT[appearance];
  const day = appearance === 'day';
  return <>
    <ForumSky appearance={appearance} />
    <color attach="background" args={[light.background]} />
    {/* Exponential haze tinted warmer than the sky, so distance reads as a
        low-sun horizon glow rather than as the scene fading into its own
        background. Night keeps the linear fall-off it already had. */}
    {day
      ? <fogExp2 attach="fog" args={['#B7C4D3', .0016]} />  // cool aerial haze so the ridge and town read against the warm sky
      : <fog attach="fog" args={[light.background, 220, 520]} />}
    <hemisphereLight args={[light.sky, light.ground, light.ambient]} />
    {/* shadow-normalBias: a 15-degree key grazes every surface it lights, and
        without it the shadow map self-shadows into stripes at that angle. */}
    <directionalLight position={day?DAY_KEY_POSITION:NIGHT_KEY_POSITION} intensity={light.strength} color={light.key}
      castShadow shadow-mapSize={[2048,2048]} shadow-camera-left={-72} shadow-camera-right={72}
      shadow-camera-top={72} shadow-camera-bottom={-72} shadow-bias={-.0005}
      shadow-normalBias={day?.03:0} />
    <directionalLight position={day?DAY_FILL_POSITION:NIGHT_FILL_POSITION} color={light.fill} intensity={light.fillStrength} />
    {appearance === 'night' && <>
      {([[55,4,0],[-55,4,0],[0,4,55],[0,4,-55]] as [number,number,number][]).map((position,i)=><pointLight key={i} position={position} color={ENV.neonAmber} intensity={40} distance={17} decay={2} />)}
      <pointLight position={[0,5,0]} color={ENV.neonAmber} intensity={30} distance={13} decay={2} />
      <pointLight position={[0,6,-20]} color={ENV.neonAmber} intensity={45} distance={15} decay={2} />
      <pointLight position={[-17,4,-4]} color={ENV.neonAmber} intensity={22} distance={10} decay={2} />
      <pointLight position={[17,4,-4]} color={ENV.neonAmber} intensity={22} distance={10} decay={2} />
    </>}
  </>;
}

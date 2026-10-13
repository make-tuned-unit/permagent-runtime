import { Suspense, useCallback, useEffect, useRef } from 'react';
import { useTexture } from '@react-three/drei';
import { GardenAssetBoundary } from './GardenAssetBoundary';
import { HABITAT_DAY_PANORAMA_URLS } from './habitatVista';
import type { Appearance } from './bakedEnvironment';

function Prepared({ appearance, onPrepared }: { appearance: Appearance; onPrepared: (appearance: Appearance) => void }) {
  useEffect(() => { onPrepared(appearance); }, [appearance, onPrepared]);
  return null;
}
function TextureReady({ url, ...props }: { url: string; appearance: Appearance; onPrepared: (appearance: Appearance) => void }) {
  useTexture(url); // Same drei resource cache as the real sky: no second download.
  return <Prepared {...props} />;
}
export function clearForumLightingPreparation(appearance: Appearance) {
  if (appearance === 'day') HABITAT_DAY_PANORAMA_URLS.forEach(url => useTexture.clear(url));
  else useTexture.clear(`${import.meta.env.BASE_URL}world/astronomy/gaia-edr3-milky-way.webp`);
}
export function ForumLightingPreparation({ appearance, compact, attempt, onPrepared, onFailure }: {
  appearance: Appearance; compact: boolean; attempt: number;
  onPrepared: (appearance: Appearance) => void; onFailure: (appearance: Appearance) => void;
}) {
  const url = appearance === 'day' ? HABITAT_DAY_PANORAMA_URLS[0] : `${import.meta.env.BASE_URL}world/astronomy/gaia-edr3-milky-way.webp`;
  const timer = useRef<ReturnType<typeof setTimeout>>();
  const settled = useRef(false);
  const prepared = useCallback((next: Appearance) => { settled.current = true; clearTimeout(timer.current); onPrepared(next); }, [onPrepared]);
  const failed = useCallback(() => { settled.current = true; clearTimeout(timer.current); onFailure(appearance); }, [onFailure, appearance]);
  useEffect(() => {
    if (!settled.current) timer.current = setTimeout(failed, 5000);
    return () => clearTimeout(timer.current);
  }, [failed]);
  if (!compact) return <Prepared appearance={appearance} onPrepared={prepared} />;
  const small = <Suspense fallback={null}><TextureReady url={HABITAT_DAY_PANORAMA_URLS[1]} appearance={appearance} onPrepared={prepared} /></Suspense>;
  return <GardenAssetBoundary key={`${appearance}:${attempt}`} onFailure={failed}>
    <GardenAssetBoundary onFailure={() => { if (appearance !== 'day') failed(); }} fallback={appearance === 'day' ? small : undefined}>
      <Suspense fallback={null}><TextureReady url={url} appearance={appearance} onPrepared={prepared} /></Suspense>
    </GardenAssetBoundary>
  </GardenAssetBoundary>;
}

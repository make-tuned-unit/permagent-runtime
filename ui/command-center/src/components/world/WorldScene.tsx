// WorldScene — thin composition file (W1-owned per WORLD_VIEW_BIBLE.md §5).
// Hall geometry lives in areas/hall/, lighting/fog/particles in atmosphere/,
// legacy furniture in props/legacy/ (W2), agents in agents/ (W3).

import { HallStructure } from './areas/hall/HallStructure';
import { MezzanineLibrary } from './areas/hall/MezzanineLibrary';
import { HallLighting, Starfield, DistantGrid, DustMotes, WorldFog } from './atmosphere/Atmosphere';
import { LegacyFurniture } from './props/legacy/WorldFurniture';

// Main scene composition
export function WorldSceneContent({
  onHoverStation,
  onClickStation,
}: {
  onHoverStation: (id: string | null) => void;
  onClickStation: (id: string) => void;
}) {
  return (
    <>
      {/* Lighting */}
      <HallLighting />

      {/* Environment */}
      <Starfield />
      <DistantGrid />

      {/* Main hall — rotunda + mezzanine library */}
      <HallStructure onHoverStation={onHoverStation} onClickStation={onClickStation} />
      <MezzanineLibrary />

      {/* Legacy station-corner furniture (W2 takes over in props/) */}
      <LegacyFurniture />

      {/* Atmosphere */}
      <DustMotes />
      <WorldFog />
    </>
  );
}

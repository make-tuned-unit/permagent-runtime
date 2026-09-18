import * as THREE from 'three';

/**
 * Runtime surface pass for the forum GLB and future forum geometry.
 *
 * The pass keeps the source materials untouched. It shares one owned clone
 * for each source material/profile pair inside a root, and the returned
 * cleanup function restores every mesh's original material before disposing
 * only those owned clones. Geometry and collision data are never changed.
 */

const SHADER_VERSION = 'forum-surface-materials-v1';
const VERTEX_MARKER = '/* forum surface world position v1 */';
const FRAGMENT_MARKER = '/* forum surface variation v1 */';

type SurfaceKind = 'accent' | 'glass' | 'water' | 'metal' | 'wood' | 'stone' | 'earth' | 'generic';

type SurfaceProfile = {
  id: SurfaceKind;
  colorVariation: number;
  roughnessVariation: number;
  broadFrequency: number;
  fineFrequency: number;
  roughnessMin: number;
  roughnessMax: number;
  /** Water only: drive the noise with a clock so the surface actually moves.
   *  Everything else on the island is static stone, timber and glass, and a
   *  crawling wall would read as a bug rather than as life. */
  waveSpeed?: number;
  /** How far the highlight bands travel, in world units per noise period. */
  waveScale?: number;
};

const SURFACE_PROFILES: Record<SurfaceKind, SurfaceProfile> = {
  accent: {
    id: 'accent', colorVariation: 0, roughnessVariation: 0,
    broadFrequency: 0, fineFrequency: 0, roughnessMin: 0, roughnessMax: 1,
  },
  glass: {
    id: 'glass', colorVariation: 0.012, roughnessVariation: 0.018,
    broadFrequency: 0.18, fineFrequency: 1.7, roughnessMin: 0.1, roughnessMax: 0.32,
  },
  water: {
    id: 'water', colorVariation: 0.03, roughnessVariation: 0.05,
    broadFrequency: 0.12, fineFrequency: 0.8, roughnessMin: 0.04, roughnessMax: 0.2,
    waveSpeed: 0.05, waveScale: 1.0,
  },
  metal: {
    id: 'metal', colorVariation: 0.018, roughnessVariation: 0.035,
    broadFrequency: 0.2, fineFrequency: 2.2, roughnessMin: 0.26, roughnessMax: 0.72,
  },
  wood: {
    id: 'wood', colorVariation: 0.065, roughnessVariation: 0.045,
    broadFrequency: 0.16, fineFrequency: 1.35, roughnessMin: 0.48, roughnessMax: 0.86,
  },
  stone: {
    id: 'stone', colorVariation: 0.045, roughnessVariation: 0.035,
    broadFrequency: 0.14, fineFrequency: 1.8, roughnessMin: 0.62, roughnessMax: 0.98,
  },
  earth: {
    id: 'earth', colorVariation: 0.085, roughnessVariation: 0.04,
    broadFrequency: 0.11, fineFrequency: 1.15, roughnessMin: 0.72, roughnessMax: 1,
  },
  generic: {
    id: 'generic', colorVariation: 0.025, roughnessVariation: 0.025,
    broadFrequency: 0.16, fineFrequency: 1.5, roughnessMin: 0.38, roughnessMax: 0.94,
  },
};

type StandardMaterial = THREE.MeshStandardMaterial;
type PhysicalMaterial = THREE.MeshPhysicalMaterial;

type MaterialChange = {
  mesh: THREE.Mesh;
  original: THREE.Material | THREE.Material[];
};

type AppliedSurfaceState = {
  cleanup: ForumSurfaceCleanup;
};

const appliedRoots = new WeakMap<THREE.Object3D, AppliedSurfaceState>();

/**
 * The water clock.
 *
 * Every animated surface material shares one value, held by reference in the
 * compiled shader's uniform object, so the whole lagoon moves together and a
 * frame costs one number per material rather than a shader recompile. The set
 * is weak-keyed by nothing — materials are disposed explicitly by
 * `disposeForumSurfaceMaterials`, and a stale uniform here would keep an object
 * alive — so `releaseForumSurfaceClock` clears it when the scene unmounts.
 */
let forumSurfaceClock = 0;

/**
 * The baked water data, shared by every water material.
 *
 * Held as uniform objects by reference, the same way the clock is, so the bake
 * can arrive after the materials have compiled and still reach them without a
 * recompile. Until it does, `uWaterInfoOn` is 0 and the shader takes the plain
 * path — a build with no bake looks exactly as it did before.
 */
const waterInfoUniforms = {
  uWaterInfo: { value: null as THREE.Texture | null },
  uWaterBounds: { value: new THREE.Vector4(0, 0, 1, 1) },
  uWaterRanges: { value: new THREE.Vector2(14, 12) },
  uWaterInfoOn: { value: 0 },
};

/** Hand the water shader its baked depth and shoreline. */
export function setForumWaterInfo(
  texture: THREE.Texture | null,
  bounds: THREE.Vector4,
  depthRange: number,
  shoreRange: number,
): void {
  waterInfoUniforms.uWaterInfo.value = texture;
  waterInfoUniforms.uWaterBounds.value = bounds;
  waterInfoUniforms.uWaterRanges.value = new THREE.Vector2(depthRange, shoreRange);
  waterInfoUniforms.uWaterInfoOn.value = texture ? 1 : 0;
}
const animatedUniforms = new Set<{ value: number }>();

/** Advance the water by `deltaSeconds`; call once per frame from the scene. */
export function advanceForumSurfaceClock(deltaSeconds: number): void {
  if (!Number.isFinite(deltaSeconds) || deltaSeconds <= 0) return;
  // Clamped so a backgrounded tab returning after a minute does not teleport
  // the swell, and wrapped so the float never loses precision in a long session.
  forumSurfaceClock = (forumSurfaceClock + Math.min(deltaSeconds, 0.1)) % 10000;
  for (const uniform of animatedUniforms) uniform.value = forumSurfaceClock;
}

/** Drop every tracked uniform — the scene is going away. */
export function releaseForumSurfaceClock(): void {
  animatedUniforms.clear();
}

export type ForumSurfaceCleanup = () => void;

const FORUM_SURFACE_HELPERS = /* glsl */ `
float forumSurfaceHash(vec3 p) {
  p = fract(p * 0.1031);
  p += dot(p, p.yzx + 33.33);
  return fract((p.x + p.y) * p.z);
}

float forumSurfaceValueNoise(vec3 p) {
  vec3 cell = floor(p);
  vec3 local = fract(p);
  local = local * local * (3.0 - 2.0 * local);
  return mix(
    mix(
      mix(forumSurfaceHash(cell + vec3(0.0, 0.0, 0.0)), forumSurfaceHash(cell + vec3(1.0, 0.0, 0.0)), local.x),
      mix(forumSurfaceHash(cell + vec3(0.0, 1.0, 0.0)), forumSurfaceHash(cell + vec3(1.0, 1.0, 0.0)), local.x),
      local.y
    ),
    mix(
      mix(forumSurfaceHash(cell + vec3(0.0, 0.0, 1.0)), forumSurfaceHash(cell + vec3(1.0, 0.0, 1.0)), local.x),
      mix(forumSurfaceHash(cell + vec3(0.0, 1.0, 1.0)), forumSurfaceHash(cell + vec3(1.0, 1.0, 1.0)), local.x),
      local.y
    ),
    local.z
  );
}
`;

function isStandardMaterial(material: THREE.Material): material is StandardMaterial {
  return (material as StandardMaterial).isMeshStandardMaterial === true;
}

function isPhysicalMaterial(material: THREE.Material): material is PhysicalMaterial {
  return (material as PhysicalMaterial).isMeshPhysicalMaterial === true;
}

function materialText(material: THREE.Material, mesh: THREE.Mesh): string {
  return `${material.name} ${mesh.name}`.toLowerCase();
}

function hasEmission(material: StandardMaterial): boolean {
  const { r, g, b } = material.emissive;
  return r * r + g * g + b * b > 0.000001;
}

function profileFor(material: StandardMaterial, mesh: THREE.Mesh): SurfaceProfile {
  const text = materialText(material, mesh);

  // Inlays, engraved intelligence, and other authored signals keep their
  // authored colour/emission. They remain in the pass so they are deliberately
  // excluded rather than accidentally softened by generic surface treatment.
  if (hasEmission(material) || /inlay|engraved|brand|logo|signage|display/.test(text)) {
    return SURFACE_PROFILES.accent;
  }
  if (/glass|glaz|window|photovoltaic|solar panel|pv\b/.test(text)) {
    return SURFACE_PROFILES.glass;
  }
  if (/water|liquid|pool|lagoon|fountain/.test(text)) {
    return SURFACE_PROFILES.water;
  }
  if (/bronze|brass|copper|steel|aluminium|aluminum|metal|iron|chrome|mullion|photovoltaic/.test(text)) {
    return SURFACE_PROFILES.metal;
  }
  if (/wood|timber|bark|louvre|louver|plank/.test(text)) {
    return SURFACE_PROFILES.wood;
  }
  if (/stone|basalt|limestone|travertine|marble|rock|concrete|paver|paving|geology|stratum/.test(text)) {
    return SURFACE_PROFILES.stone;
  }
  if (/soil|earth|ground|terrain|meadow|foliage|leaf|leaves|canopy|grass|moss|terracotta|planter|garden/.test(text)) {
    return SURFACE_PROFILES.earth;
  }
  return SURFACE_PROFILES.generic;
}

function bounded(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function addWorldPositionVarying(vertexShader: string): string {
  if (vertexShader.includes(VERTEX_MARKER)) return vertexShader;
  const declaration = `${VERTEX_MARKER}\nvarying vec3 forumSurfaceWorldPosition;`;
  const withDeclaration = vertexShader.replace('varying vec3 vViewPosition;', `varying vec3 vViewPosition;\n${declaration}`);
  const assignment = `\n\t#ifdef USE_INSTANCING\n\t\tforumSurfaceWorldPosition = ( modelMatrix * instanceMatrix * vec4( transformed, 1.0 ) ).xyz;\n\t#else\n\t\tforumSurfaceWorldPosition = ( modelMatrix * vec4( transformed, 1.0 ) ).xyz;\n\t#endif`;
  return withDeclaration.replace('#include <worldpos_vertex>', `#include <worldpos_vertex>${assignment}`);
}

function addSurfaceVariation(fragmentShader: string, profile: SurfaceProfile): string {
  const animated = (profile.waveSpeed ?? 0) > 0;
  if (fragmentShader.includes(FRAGMENT_MARKER)) return fragmentShader;
  const waterDeclarations = animated
    ? 'uniform float forumTime;\nuniform sampler2D uWaterInfo;\nuniform vec4 uWaterBounds;\nuniform vec2 uWaterRanges;\nuniform float uWaterInfoOn;\n'
    : '';
  const marker = `${FRAGMENT_MARKER}\n${waterDeclarations}${FORUM_SURFACE_HELPERS}`;
  const withHelpers = fragmentShader.replace('varying vec3 vViewPosition;', `varying vec3 vViewPosition;\nvarying vec3 forumSurfaceWorldPosition;\n${marker}`);
  // A still surface emits exactly the shader it always did — no drift
  // declarations, no extra parentheses. Stone, timber and glass make up most of
  // the island's materials, and a moving sample position on any of them would
  // read as a crawling wall; keeping their generated source byte-identical also
  // keeps the program cache, and the test that pins it, honest.
  const sampleA = animated ? '( forumSurfaceWorldPosition + forumDriftA )' : 'forumSurfaceWorldPosition';
  const sampleB = animated ? '( forumSurfaceWorldPosition + forumDriftB )' : 'forumSurfaceWorldPosition';
  const drift = animated
    ? `vec3 forumDriftA = vec3( forumTime * ${(profile.waveSpeed ?? 0).toFixed(3)}, 0.0, forumTime * ${((profile.waveSpeed ?? 0) * 0.6).toFixed(3)} );\n\tvec3 forumDriftB = vec3( forumTime * ${((profile.waveSpeed ?? 0) * -0.42).toFixed(3)}, 0.0, forumTime * ${((profile.waveSpeed ?? 0) * 0.31).toFixed(3)} );\n\t`
    : '';
  const colourValue = `${drift}float forumSurfaceColorValue = forumSurfaceValueNoise( ${sampleA} * ${profile.broadFrequency.toFixed(3)} ) * 0.35 + forumSurfaceValueNoise( ${sampleB} * ${profile.fineFrequency.toFixed(3)} ) * 0.65;`;
  const roughnessValue = `float forumSurfaceRoughnessValue = forumSurfaceValueNoise( ${sampleB} * ${profile.broadFrequency.toFixed(3)} ) * 0.35 + forumSurfaceValueNoise( ${sampleA} * ${profile.fineFrequency.toFixed(3)} ) * 0.65;`;
  const colour = `\n\t${colourValue}\n\tdiffuseColor.rgb = clamp( diffuseColor.rgb * ( 1.0 + ( forumSurfaceColorValue - 0.5 ) * ${profile.colorVariation.toFixed(4)} ), 0.0, 1.0 );`;
  const roughness = `\n\t${roughnessValue}\n\troughnessFactor = clamp( roughnessFactor + ( forumSurfaceRoughnessValue - 0.5 ) * ${profile.roughnessVariation.toFixed(4)}, 0.0, 1.0 );`;
  // Depth absorption and shoreline foam, from the baked water texture.
  //
  // Beer-Lambert: light loses more of its red end the further it travels
  // through water, which is why a lagoon is pale sand-green at the edge and
  // deep blue-green out in the middle. Doing it from a BAKED depth costs one
  // texture read instead of a depth prepass, and the depth never changes
  // because the bed does not move.
  //
  // Foam is the second half of the same story: water that meets land makes a
  // bright, moving band there, and its absence is one of the strongest tells
  // that a surface is a plane rather than a liquid.
  const waterBody = animated
    ? `
\tif ( uWaterInfoOn > 0.5 ) {
\t\tvec2 forumWaterUv = ( forumSurfaceWorldPosition.xz - uWaterBounds.xy ) * uWaterBounds.zw;
\t\tvec4 forumWater = texture2D( uWaterInfo, forumWaterUv );
\t\tif ( forumWater.a > 0.02 ) {
\t\t\tfloat forumDepthM = forumWater.r * uWaterRanges.x;
\t\t\tvec3 forumAbsorb = vec3( 0.52, 0.16, 0.09 );
\t\t\tvec3 forumDeep = vec3( 0.015, 0.085, 0.10 );
\t\t\tvec3 forumTransmit = exp( -forumAbsorb * forumDepthM );
\t\t\tdiffuseColor.rgb = mix( forumDeep, diffuseColor.rgb, forumTransmit );
\t\t\t// Foam follows DEPTH, not the distance field.
\t\t\t//
\t\t\t// Two reasons, one physical and one practical. Physically, surf is
\t\t\t// where water shoals: it breaks over the shallows, not at an arbitrary
\t\t\t// radius from the bank. Practically, the distance-to-shore channel is
\t\t\t// a chamfer over a 1.1m-per-texel mask, and this lagoon is a ring only
\t\t\t// a few texels wide — so nearly every water texel reads as "near
\t\t\t// shore" and the first two tunings foamed the whole lagoon. Depth is
\t\t\t// smooth, is measured in metres, and does not care about texel size.
\t\t\tfloat forumSurf = smoothstep( 0.5, 0.08, forumDepthM );
\t\t\tfloat forumSurfNoise = forumSurfaceValueNoise( ( forumSurfaceWorldPosition + forumDriftA * 2.0 ) * 3.5 );
\t\t\tfloat forumFoam = forumSurf * smoothstep( 0.40, 0.90, forumSurfNoise );
\t\t\tdiffuseColor.rgb = mix( diffuseColor.rgb, vec3( 0.92, 0.96, 0.97 ), forumFoam * 0.45 );
\t\t\troughnessFactor = mix( roughnessFactor, 0.85, forumFoam );
\t\t\t// Fresnel. Water seen from above is mostly the bed; water seen at a
\t\t\t// grazing angle is mostly the sky, which is why a lagoon has a bright
\t\t\t// horizon band and a dark near edge. Without this the surface reads as
\t\t\t// coloured glass at every angle. The environment map — Unreal's baked
\t\t\t// sky — is what it reflects, so the two stay consistent by construction.
\t\t\tvec3 forumViewDir = normalize( vViewPosition );
\t\t\tfloat forumFacing = clamp( dot( normalize( normal ), forumViewDir ), 0.0, 1.0 );
\t\t\tfloat forumFresnel = pow( 1.0 - forumFacing, 5.0 );
\t\t\t// Schlick with water's own reflectance at normal incidence (0.02).
\t\t\tfloat forumReflect = 0.02 + 0.98 * forumFresnel;
\t\t\troughnessFactor = mix( roughnessFactor, 0.02, forumReflect * 0.8 );
\t\t\tdiffuseColor.rgb = mix( diffuseColor.rgb, diffuseColor.rgb * 0.35, forumReflect * 0.6 );
\t\t}
\t}`
    : '';
  const withColour = withHelpers.replace('#include <color_fragment>', `#include <color_fragment>${colour}`);
  const withRoughness = withColour.replace('#include <roughnessmap_fragment>', `#include <roughnessmap_fragment>${roughness}${waterBody}`);
  if (!animated) return withRoughness;
  const scale = (profile.waveScale ?? 1).toFixed(3);
  const ripple = `\n\tfloat forumWaveA = forumSurfaceValueNoise( ( forumSurfaceWorldPosition + forumDriftA ) * ${scale} );\n\tfloat forumWaveB = forumSurfaceValueNoise( ( forumSurfaceWorldPosition.zyx + forumDriftB ) * ${(Number(scale) * 1.7).toFixed(3)} );\n\tnormal = normalize( normal + vec3( ( forumWaveA - 0.5 ) * 0.22, 0.0, ( forumWaveB - 0.5 ) * 0.22 ) );`;
  return withRoughness.replace('#include <normal_fragment_maps>', `#include <normal_fragment_maps>${ripple}`);
}

function cloneForProfile(source: StandardMaterial, profile: SurfaceProfile): StandardMaterial {
  const sourceOnBeforeCompile = source.onBeforeCompile.bind(source);
  const sourceProgramCacheKey = source.customProgramCacheKey.bind(source)();
  let replacement: StandardMaterial;

  if (profile.id === 'glass' && !isPhysicalMaterial(source)) {
    // MeshPhysicalMaterial is a MeshStandardMaterial extension. It supplies
    // transmission/IOR without throwing away source maps, colour, or emission.
    const physical = new THREE.MeshPhysicalMaterial();
    // MeshPhysicalMaterial.copy expects the advanced properties that a
    // physical source has. Calling the Standard copy layer directly keeps its
    // defaults for those optional properties while preserving every standard
    // field from a GLTF MeshStandardMaterial.
    THREE.MeshStandardMaterial.prototype.copy.call(physical, source);
    physical.defines = { ...source.defines, STANDARD: '', PHYSICAL: '' };
    replacement = physical;
  } else {
    replacement = source.clone();
  }

  if (profile.id === 'glass') {
    const glass = replacement as PhysicalMaterial;
    const sourceTransmission = isPhysicalMaterial(source) ? source.transmission : 0;
    const photovoltaic = /photovoltaic|solar panel|pv\b/i.test(source.name);
    glass.transmission = sourceTransmission > 0 ? sourceTransmission : photovoltaic ? 0.08 : 0.18;
    glass.ior = isPhysicalMaterial(source) ? source.ior : 1.5;
    // Metal coatings on PV glass can remain slightly metallic; ordinary glass
    // should retain a dielectric response.
    glass.metalness = Math.min(source.metalness, photovoltaic ? 0.35 : 0.1);
    glass.transparent = false;
  }

  replacement.roughness = bounded(source.roughness, profile.roughnessMin, profile.roughnessMax);
  if (profile.colorVariation > 0) {
    replacement.onBeforeCompile = (shader, renderer) => {
      sourceOnBeforeCompile(shader, renderer);
      shader.vertexShader = addWorldPositionVarying(shader.vertexShader);
      shader.fragmentShader = addSurfaceVariation(shader.fragmentShader, profile);
      if ((profile.waveSpeed ?? 0) > 0) {
        shader.uniforms.uWaterInfo = waterInfoUniforms.uWaterInfo;
        shader.uniforms.uWaterBounds = waterInfoUniforms.uWaterBounds;
        shader.uniforms.uWaterRanges = waterInfoUniforms.uWaterRanges;
        shader.uniforms.uWaterInfoOn = waterInfoUniforms.uWaterInfoOn;
        // The uniform object is created here and held by reference, so
        // advancing it later reaches the compiled program without recompiling.
        shader.uniforms.forumTime = { value: forumSurfaceClock };
        animatedUniforms.add(shader.uniforms.forumTime as { value: number });
      }
    };
    replacement.customProgramCacheKey = () => `${SHADER_VERSION}:${profile.id}:${sourceProgramCacheKey}`;
    replacement.needsUpdate = true;
  }
  return replacement;
}

function replacementFor(
  source: THREE.Material,
  mesh: THREE.Mesh,
  replacements: Map<THREE.Material, Map<SurfaceKind, THREE.Material>>,
  owned: Set<THREE.Material>,
): THREE.Material {
  if (!isStandardMaterial(source)) return source;
  const profile = profileFor(source, mesh);
  if (profile.id === 'accent') return source;
  let byProfile = replacements.get(source);
  if (!byProfile) {
    byProfile = new Map();
    replacements.set(source, byProfile);
  }
  const existing = byProfile.get(profile.id);
  if (existing) return existing;
  const replacement = cloneForProfile(source, profile);
  byProfile.set(profile.id, replacement);
  owned.add(replacement);
  return replacement;
}

/**
 * Apply forum surface treatment and return an idempotent cleanup function.
 * Calling this function repeatedly for the same root returns the same cleanup
 * contract. Calling cleanup twice is safe; a later application gets a fresh
 * owned material set and cannot be disposed by an older cleanup callback.
 */
export function applyForumSurfaceMaterials(root: THREE.Object3D): ForumSurfaceCleanup {
  const previous = appliedRoots.get(root);
  if (previous) return previous.cleanup;

  const replacements = new Map<THREE.Material, Map<SurfaceKind, THREE.Material>>();
  const owned = new Set<THREE.Material>();
  const changes: MaterialChange[] = [];
  let active = true;

  root.traverse(object => {
    if (!(object instanceof THREE.Mesh)) return;
    const original = object.material;
    let next: THREE.Material | THREE.Material[];
    let changed: boolean;
    if (Array.isArray(original)) {
      next = original.map(material => replacementFor(material, object, replacements, owned));
      changed = next.some((material, index) => material !== original[index]);
    } else {
      next = replacementFor(original, object, replacements, owned);
      changed = next !== original;
    }
    if (!changed) return;
    changes.push({ mesh: object, original });
    object.material = next;
  });

  const cleanup: ForumSurfaceCleanup = () => {
    if (!active) return;
    active = false;
    for (const change of changes) change.mesh.material = change.original;
    for (const material of owned) material.dispose();
    if (appliedRoots.get(root)?.cleanup === cleanup) appliedRoots.delete(root);
  };
  appliedRoots.set(root, { cleanup });
  return cleanup;
}

/** Dispose the currently applied owned material set for a root, if present. */
export function disposeForumSurfaceMaterials(root: THREE.Object3D): void {
  appliedRoots.get(root)?.cleanup();
}

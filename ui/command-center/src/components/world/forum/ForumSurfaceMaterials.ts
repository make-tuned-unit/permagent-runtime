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
    id: 'water', colorVariation: 0.018, roughnessVariation: 0.025,
    broadFrequency: 0.12, fineFrequency: 0.8, roughnessMin: 0.06, roughnessMax: 0.22,
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
  if (fragmentShader.includes(FRAGMENT_MARKER)) return fragmentShader;
  const marker = `${FRAGMENT_MARKER}\n${FORUM_SURFACE_HELPERS}`;
  const withHelpers = fragmentShader.replace('varying vec3 vViewPosition;', `varying vec3 vViewPosition;\nvarying vec3 forumSurfaceWorldPosition;\n${marker}`);
  const colourValue = `float forumSurfaceColorValue = forumSurfaceValueNoise( forumSurfaceWorldPosition * ${profile.broadFrequency.toFixed(3)} ) * 0.35 + forumSurfaceValueNoise( forumSurfaceWorldPosition * ${profile.fineFrequency.toFixed(3)} ) * 0.65;`;
  const roughnessValue = `float forumSurfaceRoughnessValue = forumSurfaceValueNoise( forumSurfaceWorldPosition * ${profile.broadFrequency.toFixed(3)} ) * 0.35 + forumSurfaceValueNoise( forumSurfaceWorldPosition * ${profile.fineFrequency.toFixed(3)} ) * 0.65;`;
  const colour = `\n\t${colourValue}\n\tdiffuseColor.rgb = clamp( diffuseColor.rgb * ( 1.0 + ( forumSurfaceColorValue - 0.5 ) * ${profile.colorVariation.toFixed(4)} ), 0.0, 1.0 );`;
  const roughness = `\n\t${roughnessValue}\n\troughnessFactor = clamp( roughnessFactor + ( forumSurfaceRoughnessValue - 0.5 ) * ${profile.roughnessVariation.toFixed(4)}, 0.0, 1.0 );`;
  const withColour = withHelpers.replace('#include <color_fragment>', `#include <color_fragment>${colour}`);
  return withColour.replace('#include <roughnessmap_fragment>', `#include <roughnessmap_fragment>${roughness}`);
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

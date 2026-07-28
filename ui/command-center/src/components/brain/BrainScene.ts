import * as THREE from 'three';
import { EffectComposer } from 'three/examples/jsm/postprocessing/EffectComposer.js';
import { RenderPass } from 'three/examples/jsm/postprocessing/RenderPass.js';
import { UnrealBloomPass } from 'three/examples/jsm/postprocessing/UnrealBloomPass.js';
import type { BrainGraph, GraphEntity, GraphMemory } from './useBrainData';

// ── Types ──────────────────────────────────────────────────────────────
interface SimNode {
  id: string;
  kind: string;
  label: string;
  note: string;
  mass: number;
  pos: THREE.Vector3;
  vel: THREE.Vector3;
  mesh: THREE.Mesh;
  pinned: boolean;
  data: GraphEntity | GraphMemory | null;
  /** Degree-derived rest scale (Obsidian pass) — hover restores to this. */
  baseScale?: number;
  /** In-scene name label (Obsidian pass 2) — billboarded, distance-faded. */
  labelSprite?: THREE.Sprite;
  /** Filter/search/hover multiplier applied to the label's distance fade. */
  labelMul?: number;
}

interface SimEdge {
  a: SimNode;
  b: SimNode;
  kind: 'self' | 'memory' | 'entity';
  k: number;
  rest: number;
  weight: number;
}

interface Pulse {
  edge: SimEdge;
  t: number;
  speed: number;
  dir: number;
}

export interface TypeFilters {
  person: boolean;
  project: boolean;
  tool: boolean;
  location: boolean;
  organization: boolean;
  concept: boolean;
  memory: boolean;
}

export interface SceneCallbacks {
  onHover: (item: { id: string; kind: string; label: string; note: string; x: number; y: number } | null) => void;
  onSelect: (item: { id: string; kind: string; label: string; note: string; data: any } | null) => void;
}

// ── Constants ──────────────────────────────────────────────────────────
const K_REP = 6.0;
const V_MAX = 1.4;
const DAMPING = 0.86;
const K_CENTER = 0.012;
const COOLING = 0.997;
const MIN_ALPHA = 0.05;

const NODE_COLORS: Record<string, number> = {
  person: 0xc8e0ff, project: 0xa855f7, tool: 0x22d3ee,
  location: 0x4ade80, organization: 0xfb923c, concept: 0x7bb7ff,
};
const MEM_FRESH = new THREE.Color(0x00d5ff);
const MEM_STALE = new THREE.Color(0x4a5468);

const EDGE_COLORS: Record<string, [number, number, number]> = {
  self: [0.6, 0.85, 1.0],
  memory: [0.0, 0.83, 1.0],
  entity: [0.66, 0.34, 0.97],
};

const PULSE_COLORS: Record<string, [number, number, number]> = {
  self: [0.85, 0.97, 1.0],
  memory: [0.4, 0.93, 1.0],
  entity: [0.78, 0.55, 1.0],
};

// ── Pulse Shaders ──────────────────────────────────────────────────────
const PULSE_VERT = `
attribute float size;
varying vec3 vCol;
uniform float uPx;
void main() {
  vCol = color;
  vec4 mv = modelViewMatrix * vec4(position, 1.0);
  gl_PointSize = size * uPx * (300.0 / -mv.z);
  gl_Position = projectionMatrix * mv;
}`;

const PULSE_FRAG = `
varying vec3 vCol;
void main() {
  vec2 c = gl_PointCoord - 0.5;
  float d = length(c);
  float a = smoothstep(0.5, 0.0, d);
  float core = smoothstep(0.25, 0.0, d);
  vec3 col = vCol + core * vec3(0.6);
  gl_FragColor = vec4(col, a);
}`;

// ── Node labels (Obsidian pass 2) ──────────────────────────────────────
// Obsidian's graph wears its names: every node carries a small label that
// fades in as you approach and dims out of a hovered node's neighborhood.
// Each label is a canvas-rendered billboard sprite; depthTest is off so a
// name is never occluded by geometry.

const LABEL_FONT = '500 26px -apple-system, "Segoe UI", system-ui, sans-serif';

function makeLabelSprite(text: string, color: string): THREE.Sprite {
  const canvas = document.createElement('canvas');
  const ctx = canvas.getContext('2d')!;
  ctx.font = LABEL_FONT;
  const metrics = ctx.measureText(text);
  const padX = 10;
  const w = Math.ceil(metrics.width) + padX * 2;
  const h = 40;
  canvas.width = w;
  canvas.height = h;
  // (Re)set state — resizing the canvas clears it.
  const c2 = canvas.getContext('2d')!;
  c2.font = LABEL_FONT;
  c2.textBaseline = 'middle';
  c2.textAlign = 'center';
  // Soft dark halo keeps names readable over bright nodes and edges.
  c2.shadowColor = 'rgba(4, 8, 16, 0.9)';
  c2.shadowBlur = 8;
  c2.fillStyle = color;
  c2.fillText(text, w / 2, h / 2);
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  const mat = new THREE.SpriteMaterial({
    map: texture,
    transparent: true,
    opacity: 0,
    depthTest: false,
    depthWrite: false,
  });
  const sprite = new THREE.Sprite(mat);
  // World-space size proportional to the canvas aspect, clamped so long
  // memory snippets don't dominate.
  const scaleY = 1.05;
  sprite.scale.set((w / h) * scaleY, scaleY, 1);
  sprite.renderOrder = 10;
  return sprite;
}

/** Trim a label for in-scene display (tooltips keep the full text). */
function labelText(kind: string, label: string): string {
  const max = kind === 'memory' ? 26 : 30;
  return label.length > max ? `${label.slice(0, max - 1)}…` : label;
}

// ── BrainScene Class ───────────────────────────────────────────────────
export class BrainScene {
  private renderer: THREE.WebGLRenderer;
  private scene: THREE.Scene;
  private camera: THREE.PerspectiveCamera;
  private container: HTMLElement;
  private callbacks: SceneCallbacks;

  private nodes: SimNode[] = [];
  private edges: SimEdge[] = [];
  private pulses: Pulse[] = [];
  private alpha = 1.0;

  private edgeLines: THREE.LineSegments | null = null;
  private pulsePoints: THREE.Points | null = null;
  private dustPoints: THREE.Points | null = null;
  private composer: EffectComposer | null = null;
  private selfMesh: THREE.Mesh | null = null;

  private orbitYaw = 0;
  private orbitPitch = 0.18;
  private orbitRadius = 32;
  private dragging = false;
  private dragMoved = false;
  private dragStart = { x: 0, y: 0 };
  private hoveredNode: SimNode | null = null;
  /** node id → connected node ids (non-self edges) for hover highlighting. */
  private adjacency = new Map<string, Set<string>>();

  private search = '';
  private typeFilter: TypeFilters = { person: true, project: true, tool: true, location: true, organization: true, concept: true, memory: true };
  private timeRange: [number, number] = [0, 1];

  private raf = 0;
  private lastTime = 0;
  private disposed = false;

  private raycaster = new THREE.Raycaster();
  private mouse = new THREE.Vector2();

  constructor(container: HTMLElement, callbacks: SceneCallbacks) {
    this.container = container;
    this.callbacks = callbacks;

    const w = container.clientWidth;
    const h = container.clientHeight;

    this.renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.renderer.setSize(w, h);
    this.renderer.setClearColor(0x000000, 0);
    container.appendChild(this.renderer.domElement);

    this.camera = new THREE.PerspectiveCamera(50, w / h, 0.1, 200);
    this.scene = new THREE.Scene();
    this.scene.fog = new THREE.FogExp2(0x0b1220, 0.006);

    // Lights
    this.scene.add(new THREE.AmbientLight(0xffffff, 0.45));
    const dir = new THREE.DirectionalLight(0xa0e0ff, 0.6);
    dir.position.set(8, 12, 6);
    this.scene.add(dir);
    const p1 = new THREE.PointLight(0xa855f7, 0.9, 60);
    p1.position.set(-12, -6, -8);
    this.scene.add(p1);
    const p2 = new THREE.PointLight(0x00d5ff, 0.7, 50);
    p2.position.set(10, -8, -6);
    this.scene.add(p2);

    // Dust
    this.createDust();

    // Post-processing: bloom for glowing globe at distance
    try {
      const composer = new EffectComposer(this.renderer);
      composer.addPass(new RenderPass(this.scene, this.camera));
      const bloom = new UnrealBloomPass(new THREE.Vector2(w, h), 0.15, 0.6, 0.92);
      composer.addPass(bloom);
      this.composer = composer;
    } catch {
      // Bloom not available — render directly
    }

    // Events
    const canvas = this.renderer.domElement;
    canvas.addEventListener('mousedown', this.onMouseDown);
    canvas.addEventListener('mousemove', this.onMouseMove);
    canvas.addEventListener('mouseup', this.onMouseUp);
    canvas.addEventListener('wheel', this.onWheel, { passive: false });

    this.lastTime = performance.now();
    this.animate();
  }

  private lastDataKey = '';

  // ── Data ─────────────────────────────────────────────────────────────
  setData(data: BrainGraph) {
    // Diff: skip full rebuild if the node and edge sets haven't changed
    const memIds = data.memories.map(m => m.id).sort().join(',');
    const entIds = data.entities.map(e => e.id).sort().join(',');
    const edgeIds = (data.edges ?? []).map(e => `${e.from}>${e.to}:${e.predicate}`).sort().join(',');
    const dataKey = `${memIds}|${entIds}|${edgeIds}`;
    if (dataKey === this.lastDataKey && this.nodes.length > 0) {
      return; // same data, skip rebuild — simulation continues undisturbed
    }

    // Preserve existing positions for nodes that survive the rebuild
    const oldPositions = new Map<string, THREE.Vector3>();
    for (const n of this.nodes) {
      oldPositions.set(n.id, n.pos.clone());
    }

    this.lastDataKey = dataKey;

    // Clear old meshes + labels
    for (const n of this.nodes) {
      this.scene.remove(n.mesh);
      if (n.labelSprite) {
        this.scene.remove(n.labelSprite);
        n.labelSprite.material.map?.dispose();
        n.labelSprite.material.dispose();
      }
    }
    if (this.edgeLines) { this.scene.remove(this.edgeLines); this.edgeLines = null; }
    if (this.pulsePoints) { this.scene.remove(this.pulsePoints); this.pulsePoints = null; }
    this.nodes = [];
    this.edges = [];
    this.pulses = [];

    // Self node
    const selfGeo = new THREE.IcosahedronGeometry(0.95, 1);
    const selfMat = new THREE.MeshPhysicalMaterial({
      color: 0xb8e8ff, emissive: 0x00d5ff, emissiveIntensity: 1.2,
      roughness: 0.15, metalness: 0.2, clearcoat: 1.0, clearcoatRoughness: 0.05,
      transparent: true, opacity: 0.95,
    });
    const selfMesh = new THREE.Mesh(selfGeo, selfMat);
    this.scene.add(selfMesh);
    this.selfMesh = selfMesh;
    const selfNode: SimNode = {
      id: 'self', kind: 'self', label: data.self.name, note: 'Self',
      mass: 6, pos: new THREE.Vector3(), vel: new THREE.Vector3(),
      mesh: selfMesh, pinned: true, data: null,
    };
    this.attachLabel(selfNode);
    this.nodes.push(selfNode);

    // Entities
    for (const ent of data.entities) {
      const kind = ent.type as string;
      const nodeColor = NODE_COLORS[kind] || 0x7bb7ff;
      let geo: THREE.BufferGeometry;
      if (kind === 'project' || kind === 'tool' || kind === 'organization') geo = new THREE.BoxGeometry(0.85, 0.85, 0.85);
      else if (kind === 'concept' || kind === 'location') geo = new THREE.OctahedronGeometry(0.7, 0);
      else geo = new THREE.SphereGeometry(0.55, 24, 18);
      const color = nodeColor;

      const mat = new THREE.MeshPhysicalMaterial({
        color, emissive: color, emissiveIntensity: 0.8,
        roughness: 0.15, transparent: true, opacity: 0.95,
      });
      const mesh = new THREE.Mesh(geo, mat);
      const oldPos = oldPositions.get(ent.id);
      if (oldPos) {
        mesh.position.copy(oldPos);
      } else {
        const angle = Math.random() * Math.PI * 2;
        const r = 6 + Math.random() * 8;
        mesh.position.set(Math.cos(angle) * r, (Math.random() - 0.5) * 4, Math.sin(angle) * r);
      }
      this.scene.add(mesh);
      const node: SimNode = {
        id: ent.id, kind, label: ent.name, note: ent.note,
        mass: 2.0, pos: mesh.position.clone(), vel: new THREE.Vector3(),
        mesh, pinned: false, data: ent,
      };
      this.attachLabel(node);
      this.nodes.push(node);
      this.edges.push({ a: selfNode, b: node, kind: 'self', k: 0.06, rest: 8.0, weight: 0.5 });
    }

    // Entity→entity connections (#495 slice 3): person→project / person→person
    // lines from graph triples. The 'entity' edge kind (color + pulse) has been
    // defined since the scene was built — this is its first producer. Endpoints
    // must both be present as nodes; the backend already dedups per triple.
    if (data.edges && data.edges.length > 0) {
      const byId = new Map<string, SimNode>(this.nodes.map(n => [n.id, n]));
      for (const e of data.edges) {
        const a = byId.get(e.from);
        const b = byId.get(e.to);
        if (a && b && a !== b) {
          this.edges.push({ a, b, kind: 'entity', k: 0.10, rest: 4.0, weight: 0.8 });
        }
      }
    }

    // Memories
    for (const mem of data.memories) {
      const radius = 0.28 + mem.weight * 0.18;
      const col = MEM_FRESH.clone().lerp(MEM_STALE, mem.age);
      const geo = new THREE.SphereGeometry(radius, 14, 12);
      const mat = new THREE.MeshPhysicalMaterial({
        color: col, emissive: col, emissiveIntensity: 0.7,
        roughness: 0.25, transparent: true, opacity: 0.92,
      });
      const mesh = new THREE.Mesh(geo, mat);
      const oldPos = oldPositions.get(mem.id);
      if (oldPos) {
        mesh.position.copy(oldPos);
      } else {
        const angle = Math.random() * Math.PI * 2;
        const r = 3 + Math.random() * 6;
        mesh.position.set(Math.cos(angle) * r, (Math.random() - 0.5) * 3, Math.sin(angle) * r);
      }
      this.scene.add(mesh);
      const node: SimNode = {
        id: mem.id, kind: 'memory', label: mem.text.slice(0, 60), note: mem.text,
        mass: 0.45 + mem.weight * 0.4, pos: mesh.position.clone(), vel: new THREE.Vector3(),
        mesh, pinned: false, data: mem,
      };
      this.attachLabel(node);
      this.nodes.push(node);

      // Connect to associated entities or self
      let linked = false;
      if (mem.ent.length > 0) {
        for (const eId of mem.ent) {
          // Match by ID, or by canonical name (ent contains "term:foo" / "cat:bar",
          // entity nodes have "e:hex" IDs but their label matches the term)
          const termName = eId.replace(/^(term|cat):/, '').toLowerCase();
          const target = this.nodes.find(n =>
            n.id === eId || (n.kind !== 'self' && n.kind !== 'memory' && n.label.toLowerCase() === termName)
          );
          if (target) {
            this.edges.push({ a: node, b: target, kind: 'memory', k: 0.12, rest: 2.4, weight: mem.weight });
            linked = true;
          }
        }
      }
      if (!linked) {
        this.edges.push({ a: node, b: selfNode, kind: 'memory', k: 0.10, rest: 5.0, weight: mem.weight });
      }
    }

    // Obsidian pass: degree-based sizing + adjacency for hover highlighting.
    // Degree counts entity+memory connections (self-spokes excluded — they
    // connect everything and carry no signal). Projects visibly accumulate:
    // every new memory link, person, or edge grows the node.
    this.adjacency.clear();
    const degree = new Map<string, number>();
    for (const e of this.edges) {
      if (e.kind === 'self') continue;
      degree.set(e.a.id, (degree.get(e.a.id) ?? 0) + 1);
      degree.set(e.b.id, (degree.get(e.b.id) ?? 0) + 1);
      if (!this.adjacency.has(e.a.id)) this.adjacency.set(e.a.id, new Set());
      if (!this.adjacency.has(e.b.id)) this.adjacency.set(e.b.id, new Set());
      this.adjacency.get(e.a.id)!.add(e.b.id);
      this.adjacency.get(e.b.id)!.add(e.a.id);
    }
    for (const n of this.nodes) {
      if (n.kind === 'self' || n.kind === 'memory') continue;
      const d = degree.get(n.id) ?? 0;
      const scale = 1 + Math.min(d, 14) * 0.09;
      n.baseScale = scale;
      n.mesh.scale.setScalar(scale);
      n.mass = 2.0 + Math.min(d, 14) * 0.2; // hubs anchor the layout
    }

    // Build edge geometry and re-apply current filter state to new nodes
    this.rebuildEdges();
    this.rebuildPulses();
    this.applyFilters();
    this.alpha = 1.0;
  }

  /** Create + register the node's in-scene name label. */
  private attachLabel(node: SimNode) {
    const color = node.kind === 'self'
      ? '#eaf6ff'
      : node.kind === 'memory'
        ? '#9fb3cc'
        : `#${new THREE.Color(NODE_COLORS[node.kind] || 0x7bb7ff).lerp(new THREE.Color(0xffffff), 0.55).getHexString()}`;
    const sprite = makeLabelSprite(labelText(node.kind, node.label), color);
    sprite.position.copy(node.pos);
    this.scene.add(sprite);
    node.labelSprite = sprite;
    node.labelMul = 1;
  }

  /** Distance-faded label pass, run every frame: names appear as you approach
   *  (hubs from further away), track their nodes, and honor filter/search/
   *  hover multipliers. Self is always named. */
  private updateLabels() {
    const cam = this.camera.position;
    for (const n of this.nodes) {
      const sprite = n.labelSprite;
      if (!sprite) continue;
      if (!n.mesh.visible) { sprite.visible = false; continue; }

      // Sit just below the node, scaled with the node itself.
      const r = (n.baseScale ?? 1) * (n.kind === 'self' ? 1.5 : n.kind === 'memory' ? 0.55 : 0.9);
      sprite.position.set(n.pos.x, n.pos.y - r - 0.55, n.pos.z);

      let alpha: number;
      if (n.kind === 'self') {
        alpha = 1;
      } else {
        const d = cam.distanceTo(n.pos);
        // Bigger nodes announce themselves from further away.
        const far = n.kind === 'memory' ? 16 : 30 + (n.baseScale ?? 1) * 8;
        const near = far - 10;
        alpha = THREE.MathUtils.clamp(1 - (d - near) / (far - near), 0, 1);
      }
      alpha *= n.labelMul ?? 1;
      sprite.material.opacity = alpha * 0.95;
      sprite.visible = alpha > 0.02;
    }
  }

  setSearch(query: string) { this.search = query.toLowerCase(); this.applyFilters(); }
  setTypeFilter(f: TypeFilters) { this.typeFilter = f; this.applyFilters(); }
  setTimeRange(r: [number, number]) { this.timeRange = r; this.applyFilters(); }

  // ── Filtering ────────────────────────────────────────────────────────
  private applyFilters() {
    for (const n of this.nodes) {
      if (n.kind === 'self') { n.mesh.visible = true; continue; }
      let visible = true;
      if (n.kind !== 'memory') {
        const filterKey = n.kind as keyof TypeFilters;
        if (filterKey in this.typeFilter && !this.typeFilter[filterKey]) visible = false;
      }
      if (n.kind === 'memory' && !this.typeFilter.memory) visible = false;
      if (n.kind === 'memory' && n.data) {
        const mem = n.data as GraphMemory;
        if (mem.age > this.timeRange[1]) visible = false;
      }
      n.mesh.visible = visible;

      // Search dimming
      const mat = n.mesh.material as THREE.MeshPhysicalMaterial;
      if (this.search && visible) {
        const matches = n.label.toLowerCase().includes(this.search) || n.note.toLowerCase().includes(this.search);
        mat.opacity = matches ? 0.95 : 0.18;
        mat.emissiveIntensity = matches ? (n.kind === 'memory' ? 0.5 : 0.6) : 0.05;
        n.labelMul = matches ? 1.4 : 0.06; // search hits keep their names lit
      } else if (visible) {
        mat.opacity = n.kind === 'memory' ? 0.92 : 0.95;
        mat.emissiveIntensity = n.kind === 'memory' ? 0.7 : 0.8;
        n.labelMul = 1;
      }
    }
    if (this.edgeLines) {
      (this.edgeLines.material as THREE.LineBasicMaterial).opacity = 0.55;
    }
    this.clearRingHighlight();
    this.rebuildEdges();
    this.alpha = Math.max(this.alpha, 0.6);
  }

  /** Obsidian-style focus: hovering a node dims everything outside its 1-ring
   *  so the neighborhood pops — nodes, names, AND edges. The base edge layer
   *  drops to a whisper while a bright overlay redraws just the ring's edges.
   *  Null restores the filter/search baseline. */
  private applyHoverHighlight(node: SimNode | null) {
    if (!node) {
      this.applyFilters(); // restores the exact baseline opacities
      return;
    }
    const ring = this.adjacency.get(node.id);
    for (const n of this.nodes) {
      if (!n.mesh.visible || n.kind === 'self') continue;
      const mat = n.mesh.material as THREE.MeshPhysicalMaterial;
      const inRing = n === node || (ring?.has(n.id) ?? false);
      mat.opacity = inRing ? 0.98 : 0.10;
      mat.emissiveIntensity = inRing ? (n === node ? 1.1 : 0.85) : 0.04;
      n.labelMul = inRing ? 1.6 : 0.04; // the neighborhood wears its names
    }
    if (this.edgeLines) {
      (this.edgeLines.material as THREE.LineBasicMaterial).opacity = 0.10;
    }
    this.buildRingHighlight(node);
  }

  /** Bright overlay for the hovered node's own edges (Obsidian keeps the
   *  focused connections lit while the rest of the web fades). */
  private ringLines: THREE.LineSegments | null = null;

  private buildRingHighlight(node: SimNode) {
    this.clearRingHighlight();
    const positions: number[] = [];
    const colors: number[] = [];
    for (const e of this.edges) {
      if (e.a !== node && e.b !== node) continue;
      if (!e.a.mesh.visible || !e.b.mesh.visible) continue;
      positions.push(e.a.pos.x, e.a.pos.y, e.a.pos.z, e.b.pos.x, e.b.pos.y, e.b.pos.z);
      const c = PULSE_COLORS[e.kind] || PULSE_COLORS.memory;
      colors.push(c[0], c[1], c[2], c[0], c[1], c[2]);
    }
    if (positions.length === 0) return;
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.Float32BufferAttribute(positions, 3));
    geo.setAttribute('color', new THREE.Float32BufferAttribute(colors, 3));
    const mat = new THREE.LineBasicMaterial({ vertexColors: true, transparent: true, opacity: 0.9 });
    this.ringLines = new THREE.LineSegments(geo, mat);
    this.ringLines.renderOrder = 5;
    this.scene.add(this.ringLines);
  }

  private clearRingHighlight() {
    if (this.ringLines) {
      this.scene.remove(this.ringLines);
      this.ringLines.geometry.dispose();
      (this.ringLines.material as THREE.Material).dispose();
      this.ringLines = null;
    }
  }

  // ── Edge Geometry ────────────────────────────────────────────────────
  private rebuildEdges() {
    if (this.edgeLines) this.scene.remove(this.edgeLines);
    const positions: number[] = [];
    const colors: number[] = [];
    for (const e of this.edges) {
      if (!e.a.mesh.visible || !e.b.mesh.visible) continue;
      positions.push(e.a.pos.x, e.a.pos.y, e.a.pos.z, e.b.pos.x, e.b.pos.y, e.b.pos.z);
      const c = EDGE_COLORS[e.kind] || EDGE_COLORS.memory;
      colors.push(c[0], c[1], c[2], c[0], c[1], c[2]);
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.Float32BufferAttribute(positions, 3));
    geo.setAttribute('color', new THREE.Float32BufferAttribute(colors, 3));
    const mat = new THREE.LineBasicMaterial({ vertexColors: true, transparent: true, opacity: 0.55 });
    this.edgeLines = new THREE.LineSegments(geo, mat);
    this.scene.add(this.edgeLines);
  }

  // ── Pulse Particles ──────────────────────────────────────────────────
  private rebuildPulses() {
    this.pulses = [];
    for (const e of this.edges) {
      const speed = 0.08 + (e.weight || 0.5) * 0.10;
      this.pulses.push({ edge: e, t: Math.random(), speed, dir: 1 });
    }
  }

  private updatePulses(dt: number) {
    if (this.pulsePoints) this.scene.remove(this.pulsePoints);
    const positions: number[] = [];
    const colors: number[] = [];
    const sizes: number[] = [];

    for (const p of this.pulses) {
      if (!p.edge.a.mesh.visible || !p.edge.b.mesh.visible) continue;
      p.t += dt * p.speed * p.dir;
      if (p.t > 1) p.t -= 1;
      if (p.t < 0) p.t += 1;

      const a = p.edge.a.pos, b = p.edge.b.pos;
      const x = a.x + (b.x - a.x) * p.t;
      const y = a.y + (b.y - a.y) * p.t;
      const z = a.z + (b.z - a.z) * p.t;
      positions.push(x, y, z);

      const c = PULSE_COLORS[p.edge.kind] || PULSE_COLORS.memory;
      colors.push(c[0], c[1], c[2]);

      const taper = 0.5 + 0.5 * Math.sin(p.t * Math.PI);
      const baseSize = p.edge.kind === 'self' ? 0.7 : p.edge.kind === 'memory' ? 0.6 : 0.5;
      sizes.push(baseSize * taper);
    }

    if (positions.length === 0) return;
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.Float32BufferAttribute(positions, 3));
    geo.setAttribute('color', new THREE.Float32BufferAttribute(colors, 3));
    geo.setAttribute('size', new THREE.Float32BufferAttribute(sizes, 1));

    const mat = new THREE.ShaderMaterial({
      vertexShader: PULSE_VERT,
      fragmentShader: PULSE_FRAG,
      uniforms: { uPx: { value: this.renderer.getPixelRatio() } },
      vertexColors: true,
      transparent: true,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
    });
    this.pulsePoints = new THREE.Points(geo, mat);
    this.scene.add(this.pulsePoints);
  }

  // ── Background Dust ──────────────────────────────────────────────────
  private createDust() {
    const count = 320;
    const positions = new Float32Array(count * 3);
    for (let i = 0; i < count; i++) {
      positions[i * 3] = (Math.random() - 0.5) * 120;
      positions[i * 3 + 1] = (Math.random() - 0.5) * 120;
      positions[i * 3 + 2] = (Math.random() - 0.5) * 120;
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.Float32BufferAttribute(positions, 3));
    const mat = new THREE.PointsMaterial({ color: 0x6080a0, size: 0.06, transparent: true, opacity: 0.20 });
    this.dustPoints = new THREE.Points(geo, mat);
    this.scene.add(this.dustPoints);
  }

  // ── Physics Step ─────────────────────────────────────────────────────
  private step(_dt: number) {
    if (this.alpha < MIN_ALPHA) return;
    this.alpha *= COOLING;

    // Repulsion
    for (let i = 0; i < this.nodes.length; i++) {
      for (let j = i + 1; j < this.nodes.length; j++) {
        const a = this.nodes[i], b = this.nodes[j];
        if (a.pinned && b.pinned) continue;
        const dx = b.pos.x - a.pos.x, dy = b.pos.y - a.pos.y, dz = b.pos.z - a.pos.z;
        const r2 = dx * dx + dy * dy + dz * dz + 0.01;
        const f = K_REP * a.mass * b.mass / r2 * this.alpha;
        const r = Math.sqrt(r2);
        const fx = f * dx / r, fy = f * dy / r, fz = f * dz / r;
        if (!a.pinned) { a.vel.x -= fx / a.mass; a.vel.y -= fy / a.mass; a.vel.z -= fz / a.mass; }
        if (!b.pinned) { b.vel.x += fx / b.mass; b.vel.y += fy / b.mass; b.vel.z += fz / b.mass; }
      }
    }

    // Springs
    for (const e of this.edges) {
      const dx = e.b.pos.x - e.a.pos.x, dy = e.b.pos.y - e.a.pos.y, dz = e.b.pos.z - e.a.pos.z;
      const r = Math.sqrt(dx * dx + dy * dy + dz * dz) + 0.01;
      const f = e.k * (r - e.rest) * this.alpha;
      const fx = f * dx / r, fy = f * dy / r, fz = f * dz / r;
      if (!e.a.pinned) { e.a.vel.x += fx / e.a.mass; e.a.vel.y += fy / e.a.mass; e.a.vel.z += fz / e.a.mass; }
      if (!e.b.pinned) { e.b.vel.x -= fx / e.b.mass; e.b.vel.y -= fy / e.b.mass; e.b.vel.z -= fz / e.b.mass; }
    }

    // Center gravity + damping + integrate
    for (const n of this.nodes) {
      if (n.pinned) continue;
      n.vel.x += -K_CENTER * n.pos.x * this.alpha;
      n.vel.y += -K_CENTER * n.pos.y * this.alpha;
      n.vel.z += -K_CENTER * n.pos.z * this.alpha;
      n.vel.multiplyScalar(DAMPING);
      n.vel.clampLength(0, V_MAX);
      n.pos.add(n.vel);
      n.mesh.position.copy(n.pos);
    }

    // Update edge positions
    if (this.edgeLines) {
      const posAttr = this.edgeLines.geometry.getAttribute('position') as THREE.BufferAttribute;
      let idx = 0;
      for (const e of this.edges) {
        if (!e.a.mesh.visible || !e.b.mesh.visible) continue;
        posAttr.setXYZ(idx++, e.a.pos.x, e.a.pos.y, e.a.pos.z);
        posAttr.setXYZ(idx++, e.b.pos.x, e.b.pos.y, e.b.pos.z);
      }
      posAttr.needsUpdate = true;
    }
  }

  // ── Mouse Handling ───────────────────────────────────────────────────
  private onMouseDown = (e: MouseEvent) => {
    this.dragging = true;
    this.dragMoved = false;
    this.dragStart = { x: e.clientX, y: e.clientY };
  };

  private onMouseMove = (e: MouseEvent) => {
    const rect = this.container.getBoundingClientRect();
    this.mouse.x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
    this.mouse.y = -((e.clientY - rect.top) / rect.height) * 2 + 1;

    if (this.dragging) {
      const dx = e.clientX - this.dragStart.x;
      const dy = e.clientY - this.dragStart.y;
      if (Math.abs(dx) > 3 || Math.abs(dy) > 3) this.dragMoved = true;
      this.orbitYaw -= dx * 0.005;
      this.orbitPitch = Math.max(-1.0, Math.min(1.2, this.orbitPitch + dy * 0.005));
      this.dragStart = { x: e.clientX, y: e.clientY };
    } else {
      // Hover picking
      this.raycaster.setFromCamera(this.mouse, this.camera);
      const meshes = this.nodes.filter(n => n.mesh.visible && n.kind !== 'self').map(n => n.mesh);
      const hits = this.raycaster.intersectObjects(meshes);
      if (hits.length > 0) {
        const mesh = hits[0].object as THREE.Mesh;
        const node = this.nodes.find(n => n.mesh === mesh);
        if (node && node !== this.hoveredNode) {
          if (this.hoveredNode) this.hoveredNode.mesh.scale.setScalar(this.hoveredNode.baseScale ?? 1);
          this.hoveredNode = node;
          node.mesh.scale.setScalar((node.baseScale ?? 1) * 1.5);
          this.applyHoverHighlight(node);
          this.renderer.domElement.style.cursor = 'pointer';
          this.callbacks.onHover({ id: node.id, kind: node.kind, label: node.label, note: node.note, x: e.clientX, y: e.clientY });
        }
      } else if (this.hoveredNode) {
        this.hoveredNode.mesh.scale.setScalar(this.hoveredNode.baseScale ?? 1);
        this.hoveredNode = null;
        this.applyHoverHighlight(null);
        this.renderer.domElement.style.cursor = 'grab';
        this.callbacks.onHover(null);
      }
    }
  };

  private onMouseUp = () => {
    if (!this.dragMoved && this.hoveredNode) {
      this.callbacks.onSelect({
        id: this.hoveredNode.id, kind: this.hoveredNode.kind,
        label: this.hoveredNode.label, note: this.hoveredNode.note,
        data: this.hoveredNode.data,
      });
    }
    this.dragging = false;
  };

  private onWheel = (e: WheelEvent) => {
    e.preventDefault();
    const zoomDelta = e.deltaY * 0.04 * (this.orbitRadius / 32);
    this.orbitRadius = Math.max(8, Math.min(180, this.orbitRadius + zoomDelta));
  };

  // ── Animation Loop ───────────────────────────────────────────────────
  private animate = () => {
    if (this.disposed) return;
    this.raf = requestAnimationFrame(this.animate);

    const now = performance.now();
    const dt = Math.min(0.05, (now - this.lastTime) / 1000);
    this.lastTime = now;

    // Auto-rotate
    if (!this.dragging) this.orbitYaw += 0.0012;

    // Update camera
    const cy = Math.cos(this.orbitYaw), sy = Math.sin(this.orbitYaw);
    const cp = Math.cos(this.orbitPitch), sp = Math.sin(this.orbitPitch);
    this.camera.position.set(
      this.orbitRadius * cp * sy,
      this.orbitRadius * sp + 4,
      this.orbitRadius * cp * cy,
    );
    this.camera.lookAt(0, 0, 0);

    // Self rotation + breathing pulse
    if (this.selfMesh) {
      this.selfMesh.rotation.y += dt * 0.3;
      this.selfMesh.rotation.x += dt * 0.1;
      const pulse = 1.0 + 0.04 * Math.sin((now / 1000) * (Math.PI * 2 / 2.5));
      this.selfMesh.scale.setScalar(pulse);
      const selfMat = this.selfMesh.material as THREE.MeshPhysicalMaterial;
      selfMat.emissiveIntensity = 1.2 + 0.12 * Math.sin((now / 1000) * (Math.PI * 2 / 2.5));
    }

    // Distance-aware emissive boost — nodes brighten when pulled back.
    // Suspended while hovering: the per-frame reset used to overwrite the
    // hover dim, so out-of-ring nodes never actually faded.
    if (!this.hoveredNode) {
      const zoomFactor = Math.max(1.0, this.orbitRadius / 32);
      const distanceBoost = Math.min(1.3, zoomFactor);
      for (const n of this.nodes) {
        if (n.kind === 'self' || !n.mesh.visible) continue;
        const mat = n.mesh.material as THREE.MeshPhysicalMaterial;
        const base = n.kind === 'memory' ? 0.7 : 0.8;
        mat.emissiveIntensity = base * (n.kind === 'memory' ? distanceBoost : 1.0 + (distanceBoost - 1) * 0.5);
      }
    }

    this.step(dt);
    this.updateLabels();
    // Keep the hover ring's bright edges glued to their moving endpoints.
    if (this.hoveredNode && this.ringLines) {
      const posAttr = this.ringLines.geometry.getAttribute('position') as THREE.BufferAttribute;
      let idx = 0;
      const hovered = this.hoveredNode;
      for (const e of this.edges) {
        if (e.a !== hovered && e.b !== hovered) continue;
        if (!e.a.mesh.visible || !e.b.mesh.visible) continue;
        if (idx + 1 >= posAttr.count) break;
        posAttr.setXYZ(idx++, e.a.pos.x, e.a.pos.y, e.a.pos.z);
        posAttr.setXYZ(idx++, e.b.pos.x, e.b.pos.y, e.b.pos.z);
      }
      posAttr.needsUpdate = true;
    }
    this.updatePulses(dt);
    if (this.composer) {
      this.composer.render();
    } else {
      this.renderer.render(this.scene, this.camera);
    }
  };

  // ── Resize ───────────────────────────────────────────────────────────
  resize() {
    const w = this.container.clientWidth;
    const h = this.container.clientHeight;
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.renderer.setSize(w, h);
    if (this.composer) this.composer.setSize(w, h);
  }

  // ── Dispose ──────────────────────────────────────────────────────────
  dispose() {
    this.disposed = true;
    cancelAnimationFrame(this.raf);
    const canvas = this.renderer.domElement;
    canvas.removeEventListener('mousedown', this.onMouseDown);
    canvas.removeEventListener('mousemove', this.onMouseMove);
    canvas.removeEventListener('mouseup', this.onMouseUp);
    canvas.removeEventListener('wheel', this.onWheel);

    this.scene.traverse(obj => {
      if (obj instanceof THREE.Sprite) {
        obj.material.map?.dispose();
        obj.material.dispose();
        return;
      }
      if (obj instanceof THREE.Mesh) {
        obj.geometry.dispose();
        if (Array.isArray(obj.material)) obj.material.forEach(m => m.dispose());
        else obj.material.dispose();
      }
    });
    this.renderer.dispose();
    if (canvas.parentElement) canvas.parentElement.removeChild(canvas);
  }
}

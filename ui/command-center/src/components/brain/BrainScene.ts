import * as THREE from 'three';
import type { BrainGraph, GraphEntity, GraphMemory } from './useBrainData';

// ── Types ──────────────────────────────────────────────────────────────
interface SimNode {
  id: string;
  kind: 'self' | 'person' | 'project' | 'topic' | 'memory';
  label: string;
  note: string;
  mass: number;
  pos: THREE.Vector3;
  vel: THREE.Vector3;
  mesh: THREE.Mesh;
  pinned: boolean;
  data: GraphEntity | GraphMemory | null;
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
  topic: boolean;
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
  person: 0xc8e0ff, project: 0xa855f7, topic: 0x7bb7ff,
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
  private selfMesh: THREE.Mesh | null = null;

  private orbitYaw = 0;
  private orbitPitch = 0.18;
  private orbitRadius = 32;
  private dragging = false;
  private dragMoved = false;
  private dragStart = { x: 0, y: 0 };
  private hoveredNode: SimNode | null = null;

  private search = '';
  private typeFilter: TypeFilters = { person: true, project: true, topic: true, memory: true };
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

    this.renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.renderer.setSize(w, h);
    this.renderer.setClearColor(0x070b14);
    container.appendChild(this.renderer.domElement);

    this.camera = new THREE.PerspectiveCamera(50, w / h, 0.1, 200);
    this.scene = new THREE.Scene();
    this.scene.fog = new THREE.FogExp2(0x070b14, 0.018);

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

    // Events
    const canvas = this.renderer.domElement;
    canvas.addEventListener('mousedown', this.onMouseDown);
    canvas.addEventListener('mousemove', this.onMouseMove);
    canvas.addEventListener('mouseup', this.onMouseUp);
    canvas.addEventListener('wheel', this.onWheel, { passive: false });

    this.lastTime = performance.now();
    this.animate();
  }

  // ── Data ─────────────────────────────────────────────────────────────
  setData(data: BrainGraph) {
    // Clear old meshes
    for (const n of this.nodes) this.scene.remove(n.mesh);
    if (this.edgeLines) { this.scene.remove(this.edgeLines); this.edgeLines = null; }
    if (this.pulsePoints) { this.scene.remove(this.pulsePoints); this.pulsePoints = null; }
    this.nodes = [];
    this.edges = [];
    this.pulses = [];

    // Self node
    const selfGeo = new THREE.IcosahedronGeometry(0.95, 1);
    const selfMat = new THREE.MeshPhysicalMaterial({
      color: 0xb8e8ff, emissive: 0x00d5ff, emissiveIntensity: 2.4,
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
    this.nodes.push(selfNode);

    // Entities
    for (const ent of data.entities) {
      const kind = ent.type as 'person' | 'project' | 'topic';
      const color = NODE_COLORS[kind] || 0x7bb7ff;
      let geo: THREE.BufferGeometry;
      if (kind === 'project') geo = new THREE.BoxGeometry(0.85, 0.85, 0.85);
      else if (kind === 'topic') geo = new THREE.OctahedronGeometry(0.7, 0);
      else geo = new THREE.SphereGeometry(0.55, 24, 18);

      const mat = new THREE.MeshPhysicalMaterial({
        color, emissive: color, emissiveIntensity: 1.6,
        roughness: 0.15, transparent: true, opacity: 0.95,
      });
      const mesh = new THREE.Mesh(geo, mat);
      const angle = Math.random() * Math.PI * 2;
      const r = 6 + Math.random() * 8;
      mesh.position.set(Math.cos(angle) * r, (Math.random() - 0.5) * 4, Math.sin(angle) * r);
      this.scene.add(mesh);
      const node: SimNode = {
        id: ent.id, kind, label: ent.name, note: ent.note,
        mass: 2.0, pos: mesh.position.clone(), vel: new THREE.Vector3(),
        mesh, pinned: false, data: ent,
      };
      this.nodes.push(node);
      this.edges.push({ a: selfNode, b: node, kind: 'self', k: 0.06, rest: 8.0, weight: 0.5 });
    }

    // Memories
    for (const mem of data.memories) {
      const radius = 0.14 + mem.weight * 0.16;
      const col = MEM_FRESH.clone().lerp(MEM_STALE, mem.age);
      const geo = new THREE.SphereGeometry(radius, 14, 12);
      const mat = new THREE.MeshPhysicalMaterial({
        color: col, emissive: col, emissiveIntensity: 1.5,
        roughness: 0.25, transparent: true, opacity: 0.92,
      });
      const mesh = new THREE.Mesh(geo, mat);
      const angle = Math.random() * Math.PI * 2;
      const r = 3 + Math.random() * 6;
      mesh.position.set(Math.cos(angle) * r, (Math.random() - 0.5) * 3, Math.sin(angle) * r);
      this.scene.add(mesh);
      const node: SimNode = {
        id: mem.id, kind: 'memory', label: mem.text.slice(0, 60), note: mem.text,
        mass: 0.45 + mem.weight * 0.4, pos: mesh.position.clone(), vel: new THREE.Vector3(),
        mesh, pinned: false, data: mem,
      };
      this.nodes.push(node);

      // Connect to associated entities or self
      if (mem.ent.length > 0) {
        for (const eId of mem.ent) {
          const target = this.nodes.find(n => n.id === eId);
          if (target) this.edges.push({ a: node, b: target, kind: 'memory', k: 0.12, rest: 2.4, weight: mem.weight });
        }
      } else {
        this.edges.push({ a: node, b: selfNode, kind: 'memory', k: 0.10, rest: 5.0, weight: mem.weight });
      }
    }

    // Build edge geometry
    this.rebuildEdges();
    this.rebuildPulses();
    this.alpha = 1.0;
  }

  setSearch(query: string) { this.search = query.toLowerCase(); this.applyFilters(); }
  setTypeFilter(f: TypeFilters) { this.typeFilter = f; this.applyFilters(); }
  setTimeRange(r: [number, number]) { this.timeRange = r; this.applyFilters(); }

  // ── Filtering ────────────────────────────────────────────────────────
  private applyFilters() {
    for (const n of this.nodes) {
      if (n.kind === 'self') { n.mesh.visible = true; continue; }
      let visible = true;
      if (n.kind !== 'memory' && !this.typeFilter[n.kind as keyof TypeFilters]) visible = false;
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
        mat.emissiveIntensity = matches ? (n.kind === 'memory' ? 0.6 : 0.75) : 0.1;
      } else if (visible) {
        mat.opacity = n.kind === 'memory' ? 0.92 : 0.95;
        mat.emissiveIntensity = n.kind === 'memory' ? 1.5 : 1.6;
      }
    }
    this.rebuildEdges();
    this.alpha = Math.max(this.alpha, 0.6);
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
    const mat = new THREE.PointsMaterial({ color: 0x6080a0, size: 0.05, transparent: true, opacity: 0.45 });
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
          if (this.hoveredNode) this.hoveredNode.mesh.scale.setScalar(1);
          this.hoveredNode = node;
          node.mesh.scale.setScalar(1.25);
          this.renderer.domElement.style.cursor = 'pointer';
          this.callbacks.onHover({ id: node.id, kind: node.kind, label: node.label, note: node.note, x: e.clientX, y: e.clientY });
        }
      } else if (this.hoveredNode) {
        this.hoveredNode.mesh.scale.setScalar(1);
        this.hoveredNode = null;
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
    this.orbitRadius = Math.max(10, Math.min(70, this.orbitRadius + e.deltaY * 0.04));
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

    // Self rotation
    if (this.selfMesh) {
      this.selfMesh.rotation.y += dt * 0.3;
      this.selfMesh.rotation.x += dt * 0.1;
    }

    this.step(dt);
    this.updatePulses(dt);
    this.renderer.render(this.scene, this.camera);
  };

  // ── Resize ───────────────────────────────────────────────────────────
  resize() {
    const w = this.container.clientWidth;
    const h = this.container.clientHeight;
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
    this.renderer.setSize(w, h);
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

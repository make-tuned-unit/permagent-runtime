import { Component, Suspense, useMemo, type ReactNode } from 'react';
import { useGLTF } from '@react-three/drei';
import { Mesh } from 'three';

export const VAULT_ASSET = `${import.meta.env.BASE_URL}world/observatory-vault.glb`;

// A Canvas-safe boundary: the shared DOM ErrorBoundary cannot render inside R3F.
export class VaultBoundary extends Component<
  { children: ReactNode; fallback: ReactNode }, { failed: boolean }
> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch() { console.warn('[world] Blender vault unavailable; using procedural hall'); }
  render() { return this.state.failed ? this.props.fallback : this.props.children; }
}

function VaultAsset() {
  const { scene } = useGLTF(VAULT_ASSET);
  const instance = useMemo(() => {
    // Shared loader cache owns geometry/materials; mounts only own transforms.
    const clone = scene.clone(true);
    clone.userData.blenderVault = true;
    clone.traverse((object) => {
      if (object instanceof Mesh) {
        object.receiveShadow = true;
        // Keep the existing single shadow map cheap: decorative vault ribs do
        // not add a second shadow render of 75k triangles every invalidation.
        object.castShadow = false;
        // Architecture must not swallow clicks on agents/stations behind it.
        object.raycast = () => {};
      }
    });
    return clone;
  }, [scene]);
  return <primitive object={instance} dispose={null} />;
}

export function BlenderVault({ fallback }: { fallback: ReactNode }) {
  return (
    <VaultBoundary fallback={fallback}>
      <Suspense fallback={fallback}><VaultAsset /></Suspense>
    </VaultBoundary>
  );
}

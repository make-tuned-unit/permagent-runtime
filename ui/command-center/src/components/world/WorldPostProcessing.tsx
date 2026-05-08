import { Bloom, EffectComposer, Noise } from '@react-three/postprocessing';
import { BlendFunction, KernelSize } from 'postprocessing';

export function WorldPostProcessing() {
  return (
    <EffectComposer>
      <Bloom
        intensity={0.6}
        luminanceThreshold={0.85}
        luminanceSmoothing={0.3}
        kernelSize={KernelSize.MEDIUM}
      />
      <Noise
        premultiply
        blendFunction={BlendFunction.ADD}
        opacity={0.15}
      />
    </EffectComposer>
  );
}

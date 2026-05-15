import { Bloom, EffectComposer, Noise } from '@react-three/postprocessing';
import { BlendFunction, KernelSize } from 'postprocessing';

export function WorldPostProcessing() {
  return (
    <EffectComposer>
      <Bloom
        intensity={0.8}
        luminanceThreshold={0.4}
        luminanceSmoothing={0.4}
        kernelSize={KernelSize.LARGE}
      />
      <Noise
        premultiply
        blendFunction={BlendFunction.ADD}
        opacity={0.12}
      />
    </EffectComposer>
  );
}

import { MeshoptDecoder } from 'three/examples/jsm/libs/meshopt_decoder.module.js';
type DecodeArgs = Parameters<typeof MeshoptDecoder.decodeGltfBufferAsync>;
const scope = self as unknown as {
  onmessage: (event: MessageEvent<{ id: number; args: DecodeArgs }>) => void;
  postMessage(message: unknown, transfer?: Transferable[]): void;
};
scope.onmessage = async ({ data: { id, args } }) => {
  try {
    const result = await MeshoptDecoder.decodeGltfBufferAsync(...args);
    scope.postMessage({ id, result }, [result.buffer as ArrayBuffer]);
  } catch (error) { scope.postMessage({ id, error: String(error) }); }
};

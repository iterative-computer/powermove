/** Decode pm_model_import_packed's mesh scalars while leaving all scene and
 * extension metadata as ordinary JSON. Every Float32/Uint32 view shares the
 * caller-owned packet buffer, which must be copied out of WASM first. */
export function decodeModelPacket(packet: Uint8Array): any {
  // TypedArray views require four-byte alignment. The WASM wrapper supplies
  // an owned, aligned copy; support arbitrary callers without misreading.
  if (packet.byteOffset % 4 !== 0) packet = packet.slice();
  if (packet.byteLength < 4) throw new Error('Invalid packed model header');
  const view = new DataView(packet.buffer, packet.byteOffset, packet.byteLength);
  const headerLength = view.getUint32(0, true);
  const payloadStart = Math.ceil((4 + headerLength) / 4) * 4;
  if (payloadStart > packet.byteLength) throw new Error('Invalid packed model length');
  const scene = JSON.parse(new TextDecoder().decode(packet.subarray(4, 4 + headerLength)));
  const payloadLength = packet.byteLength - payloadStart;

  const field = (value: any, components: number, type: 'f32' | 'u32') => {
    if (value == null || Array.isArray(value)) return value;
    const marker = value?.packed;
    const { offset, length } = marker ?? {};
    if (marker?.components !== components || marker?.type !== type ||
        !Number.isSafeInteger(offset) || offset < 0 || offset % 4 !== 0 ||
        !Number.isSafeInteger(length) || length < 0 || length % components !== 0 ||
        length * 4 > payloadLength - offset) {
      throw new Error('Invalid packed model attribute');
    }
    const start = packet.byteOffset + payloadStart + offset;
    return type === 'f32'
      ? new Float32Array(packet.buffer, start, length)
      : new Uint32Array(packet.buffer, start, length);
  };

  for (const mesh of scene.meshes ?? []) {
    for (const primitive of mesh.primitives ?? []) {
      primitive.positions = field(primitive.positions, 3, 'f32');
      primitive.normals = field(primitive.normals, 3, 'f32');
      for (const key of ['uv', 'uv1', 'uv2', 'uv3']) primitive[key] = field(primitive[key], 2, 'f32');
      primitive.colors = field(primitive.colors, 4, 'f32');
      primitive.tangents = field(primitive.tangents, 4, 'f32');
      primitive.joints = field(primitive.joints, 4, 'u32');
      primitive.weights = field(primitive.weights, 4, 'f32');
      primitive.indices = field(primitive.indices, 1, 'u32');
      for (const target of primitive.targets ?? []) {
        target.position = field(target.position, 3, 'f32');
        target.normal = field(target.normal, 3, 'f32');
      }
    }
  }
  return scene;
}

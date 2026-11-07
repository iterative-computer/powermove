/** A console made of named parts: Console → [Screen, JoyCon L, JoyCon R → Stick R]. Boxes, no UVs. */
export function partsGlb(): Uint8Array<ArrayBuffer> {
  const box = (w: number, h: number, d: number) => {
    const [x, y, z] = [w / 2, h / 2, d / 2];
    const faces: [number[], number[][]][] = [
      [[1, 0, 0], [[x, -y, -z], [x, y, -z], [x, y, z], [x, -y, z]]], [[-1, 0, 0], [[-x, -y, z], [-x, y, z], [-x, y, -z], [-x, -y, -z]]],
      [[0, 1, 0], [[-x, y, -z], [-x, y, z], [x, y, z], [x, y, -z]]], [[0, -1, 0], [[-x, -y, z], [-x, -y, -z], [x, -y, -z], [x, -y, z]]],
      [[0, 0, 1], [[-x, -y, z], [x, -y, z], [x, y, z], [-x, y, z]]], [[0, 0, -1], [[x, -y, -z], [-x, -y, -z], [-x, y, -z], [x, y, -z]]]];
    const positions: number[] = [], normals: number[] = [], indices: number[] = [];
    for (const [n, vs] of faces) { const b = positions.length / 3; for (const v of vs) { positions.push(...v); normals.push(...n); } indices.push(b, b + 1, b + 2, b, b + 2, b + 3); }
    return { positions, normals, indices, min: [-x, -y, -z], max: [x, y, z] };
  };
  const parts: [string, ReturnType<typeof box>, number[], number][] = [
    ['Screen', box(3, 1.8, .25), [0, 0, 0], 0], ['JoyCon L', box(.6, 1.8, .3), [-1.82, 0, 0], 1],
    ['JoyCon R', box(.6, 1.8, .3), [1.82, 0, 0], 2], ['Stick R', box(.22, .22, .12), [0, .45, .2], 0]];
  const chunks: number[][] = [], views: any[] = [], accessors: any[] = [], meshes: any[] = [], nodes: any[] = [];
  let offset = 0;
  const add = (data: number[], type: string, componentType: number, target: number, extra: any = {}) => {
    const bytes = componentType === 5123 ? data.length * 2 : data.length * 4, padded = Math.ceil(bytes / 4) * 4;
    views.push({ buffer: 0, byteOffset: offset, byteLength: bytes, target }); chunks.push(data); offset += padded;
    accessors.push({ bufferView: views.length - 1, componentType, count: data.length / (type === 'VEC3' ? 3 : 1), type, ...extra });
    return accessors.length - 1;
  };
  for (const [name, geometry, translation, material] of parts) {
    const position = add(geometry.positions, 'VEC3', 5126, 34962, { min: geometry.min, max: geometry.max });
    const normal = add(geometry.normals, 'VEC3', 5126, 34962), index = add(geometry.indices, 'SCALAR', 5123, 34963);
    meshes.push({ name, primitives: [{ attributes: { POSITION: position, NORMAL: normal }, indices: index, material }] });
    nodes.push({ name, mesh: meshes.length - 1, translation });
  }
  nodes[2].children = [3]; nodes.push({ name: 'Console', children: [0, 1, 2] });
  const binary = new Uint8Array(offset), binaryView = new DataView(binary.buffer);
  views.forEach((view, i) => chunks[i]!.forEach((value, j) => accessors[i].componentType === 5123
    ? binaryView.setUint16(view.byteOffset + j * 2, value, true) : binaryView.setFloat32(view.byteOffset + j * 4, value, true)));
  const json = JSON.stringify({ asset: { version: '2.0' }, scene: 0, scenes: [{ nodes: [4] }], nodes, meshes,
    materials: [{ name: 'Body', pbrMetallicRoughness: { baseColorFactor: [.12, .12, .14, 1] } }, { name: 'Blue', pbrMetallicRoughness: { baseColorFactor: [.35, .75, .85, 1] } }, { name: 'Red', pbrMetallicRoughness: { baseColorFactor: [.95, .38, .3, 1] } }],
    accessors, bufferViews: views, buffers: [{ byteLength: binary.byteLength }] });
  const text = new TextEncoder().encode(json.padEnd(Math.ceil(json.length / 4) * 4, ' '));
  const bytes = new Uint8Array(28 + text.length + binary.byteLength), view = new DataView(bytes.buffer);
  for (const [at, value] of [[0, 0x46546c67], [4, 2], [8, bytes.length], [12, text.length], [16, 0x4e4f534a], [20 + text.length, binary.byteLength], [24 + text.length, 0x004e4942]]) view.setUint32(at!, value!, true);
  bytes.set(text, 20); bytes.set(binary, 28 + text.length); return bytes;
}

/** Tiny self-contained screen, with glTF's top-left UV origin. No external assets. */
export function screenGlb(): Uint8Array<ArrayBuffer> {
  const positions = [-1,-1,0, 1,-1,0, -1,1,0, 1,-1,0, 1,1,0, -1,1,0];
  const uvs = [0,1, 1,1, 0,0, 1,1, 1,0, 0,0];
  const binary = new Float32Array([...positions,...uvs]);
  const json = JSON.stringify({asset:{version:'2.0'},scene:0,scenes:[{nodes:[0]}],nodes:[{mesh:0}],
    materials:[{name:'Screen',pbrMetallicRoughness:{baseColorFactor:[1,1,1,1],metallicFactor:0,roughnessFactor:1},doubleSided:true}],
    meshes:[{primitives:[{attributes:{POSITION:0,TEXCOORD_0:1},material:0}]}],buffers:[{byteLength:binary.byteLength}],
    bufferViews:[{buffer:0,byteOffset:0,byteLength:72},{buffer:0,byteOffset:72,byteLength:48}],
    accessors:[{bufferView:0,componentType:5126,count:6,type:'VEC3',min:[-1,-1,0],max:[1,1,0]},
      {bufferView:1,componentType:5126,count:6,type:'VEC2'}]});
  const text=new TextEncoder().encode(json.padEnd(Math.ceil(json.length/4)*4,' '));
  const bytes=new Uint8Array(28+text.length+binary.byteLength),view=new DataView(bytes.buffer);
  for(const [offset,value] of [[0,0x46546c67],[4,2],[8,bytes.length],[12,text.length],[16,0x4e4f534a],
    [20+text.length,binary.byteLength],[24+text.length,0x004e4942]])view.setUint32(offset!,value!,true);
  bytes.set(text,20);bytes.set(new Uint8Array(binary.buffer),28+text.length);return bytes;
}

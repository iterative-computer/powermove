"""Generate tiny valid cube fixtures in a supplied scratch folder (no binary Git files)."""
import json, struct, sys, zlib
from pathlib import Path
out = Path(sys.argv[1]); out.mkdir(parents=True, exist_ok=True)
v = [(-.5,-.5,-.5),(.5,-.5,-.5),(.5,.5,-.5),(-.5,.5,-.5),(-.5,-.5,.5),(.5,-.5,.5),(.5,.5,.5),(-.5,.5,.5)]
i = [0,2,1,0,3,2,4,5,6,4,6,7,0,1,5,0,5,4,3,7,6,3,6,2,0,4,7,0,7,3,1,2,6,1,6,5]
b = b''.join(struct.pack('<3f',*p) for p in v) + struct.pack('<36H',*i)
d = {'asset':{'version':'2.0'},'scene':0,'scenes':[{'nodes':[0]}],'nodes':[{'mesh':0}],'buffers':[{'byteLength':len(b)}],'bufferViews':[{'buffer':0,'byteOffset':0,'byteLength':96},{'buffer':0,'byteOffset':96,'byteLength':72}],'accessors':[{'bufferView':0,'componentType':5126,'count':8,'type':'VEC3','min':[-.5]*3,'max':[.5]*3},{'bufferView':1,'componentType':5123,'count':36,'type':'SCALAR'}],'meshes':[{'primitives':[{'attributes':{'POSITION':0},'indices':1}]}]}
j=json.dumps(d,separators=(',',':')).encode();j+=b' '*(-len(j)%4)
(out/'cube.glb').write_bytes(struct.pack('<4sII',b'glTF',2,28+len(j)+len(b))+struct.pack('<I4s',len(j),b'JSON')+j+struct.pack('<I4s',len(b),b'BIN\0')+b)
d['buffers'][0]['uri']='cube.bin'
(out/'textures').mkdir(exist_ok=True)
def chunk(kind, data):
    return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
png = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>2I5B',1,1,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(b'\0\xff\0\0\xff'))+chunk(b'IEND',b'')
(out/'textures/red.png').write_bytes(png)
d['images']=[{'uri':'textures/red.png'}]
(out/'cube.gltf').write_text(json.dumps(d));(out/'cube.bin').write_bytes(b)
(out/'cube.mtl').write_text('newmtl red\nKd 1 0.2 0.1\n')
(out/'cube.obj').write_text('mtllib cube.mtl\n'+''.join('v %g %g %g\n'%p for p in v)+'usemtl red\n'+''.join('f %d %d %d\n'%tuple(x+1 for x in i[n:n+3]) for n in range(0,len(i),3)))

# Existing point-cloud loaders, exercised through the same model path.
(out/'points.ply').write_bytes(b'ply\nformat binary_little_endian 1.0\nelement vertex 2\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nend_header\n'+b''.join(struct.pack('<3f3B',*p,255,80,40) for p in [(0,0,0),(1,1,1)]))
(out/'points.splat').write_bytes(b''.join(struct.pack('<6f8B',*p,.1,.1,.1,255,80,40,255,255,128,128,128) for p in [(0,0,0),(1,1,1)]))

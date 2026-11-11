import { describe, expect, it } from 'vitest';
import * as THREE from 'three';
import { PARTS_ROOT_FLAG, applyParts, captureRestPose, groupedParts, listParts, partKeyOf, partNode, partOffsetsForLocal, partsRoot, restoreParts } from './model-parts';
import { createObject, parseScene, validateSceneChannel } from './schema';
import { layer3DProperties } from './layers';

/** root → normalization (½ scale) → file scene → Console → [Screen, JoyCon R → Stick] */
function model() {
  const root = new THREE.Group(), normalization = new THREE.Group(), scene = new THREE.Group();
  normalization.scale.setScalar(.5); scene.userData[PARTS_ROOT_FLAG] = true;
  const named = (name: string, mesh = true) => { const o = mesh ? new THREE.Mesh(new THREE.BoxGeometry()) : new THREE.Group(); o.userData.name = name; o.name = name.replaceAll(' ', '_'); return o; };
  const console_ = named('Console', false), screen = named('Screen'), joycon = named('JoyCon R'), stick = named('Stick');
  // A parent with a non-uniform scale and a turn, as exported files often have.
  console_.scale.set(4, 4, 4); console_.rotation.y = Math.PI / 2;
  joycon.position.set(.5, 0, 0); stick.position.set(0, .1, 0);
  const primitive = new THREE.Mesh(new THREE.BoxGeometry()); // unnamed, moves with its node
  joycon.add(stick, primitive); console_.add(screen, joycon); scene.add(console_); normalization.add(scene); root.add(normalization);
  captureRestPose(root);
  return { root, scene, console_, screen, joycon, stick, primitive };
}
const modelPoint = (root: THREE.Object3D, node: THREE.Object3D) => {
  root.updateMatrixWorld(true);
  return node.getWorldPosition(new THREE.Vector3()).applyMatrix4(root.matrixWorld.clone().invert());
};

describe('model parts', () => {
  it('groups siblings without changing file node keys or individual animation', () => {
    const {root,scene,joycon,screen,stick}=model();
    applyParts(root,{'0_1':{y:.2}});const baseline=[screen,joycon,stick].map(node=>modelPoint(root,node));restoreParts(root);
    const pivot=new THREE.Vector3(.1,0,0),group={name:'Controllers',parent:'0',members:['0_0','0_1'],pivot:pivot.toArray()};
    const structure={group_rig:group};
    const tree=groupedParts(listParts(scene),structure);
    expect(tree.map(part=>[part.key,part.parent])).toEqual([['0',null],['group_rig','0'],['0_0','group_rig'],['0_1','group_rig'],['0_1_0','0_1']]);
    applyParts(root,{'0_1':{y:.2},group_rig:{x:.3,rz:90,scale:200}},structure);
    for(const [i,node] of [screen,joycon,stick].entries()){
      const expected=baseline[i]!.clone().sub(pivot).multiplyScalar(2).applyAxisAngle(new THREE.Vector3(0,0,1),Math.PI/2).add(pivot).add(new THREE.Vector3(.3,0,0));
      expect(modelPoint(root,node).distanceTo(expected)).toBeCloseTo(0,7);
    }
    expect(partNode(scene,'0_1')).toBe(joycon);expect(partKeyOf(scene,stick)).toBe('0_1_0');
    joycon.updateWorldMatrix(true,false);
    const individual=partOffsetsForLocal(root,joycon,joycon.matrix.clone());expect(individual.y).toBeCloseTo(.2);expect(individual.scale).toBeCloseTo(100);
    const handle=partNode(scene,'group_rig')!;
    expect(partOffsetsForLocal(root,handle,handle.matrix.clone())).toMatchObject({x:expect.closeTo(.3),rz:expect.closeTo(90),scale:expect.closeTo(200)});
    restoreParts(root);applyParts(root,{'0_1':{y:.4},group_rig:{x:.6}},structure);
    expect(modelPoint(root,joycon).y-baseline[1]!.y).toBeCloseTo(.2);
    restoreParts(root);applyParts(root,{});
    expect(partNode(scene,'group_rig')).toBeNull();expect(modelPoint(root,joycon).x).toBeCloseTo(baseline[1]!.x);
  });

  it('composes nested group handles and retains a static ungroup transform exactly',()=>{
    const {root,scene,joycon}=model(),position=modelPoint(root,joycon);
    const structure={group_inner:{name:'Inner',parent:'group_outer',members:['0_0','0_1'],pivot:[0,0,0]},group_outer:{name:'Outer',parent:'0',members:['group_inner'],pivot:[0,0,0]}};
    applyParts(root,{group_inner:{x:.2,rz:30},group_outer:{y:.4,rz:45}},structure);
    const handle=partNode(scene,'group_inner')!;
    const offsets=partOffsetsForLocal(root,handle,handle.matrix.clone());expect(offsets.x).toBeCloseTo(.2);expect(offsets.rz).toBeCloseTo(30);
    const expected=position.clone().applyAxisAngle(new THREE.Vector3(0,0,1),Math.PI/6).add(new THREE.Vector3(.2,0,0)).applyAxisAngle(new THREE.Vector3(0,0,1),Math.PI/4).add(new THREE.Vector3(0,.4,0));
    expect(modelPoint(root,joycon).distanceTo(expected)).toBeCloseTo(0);
    restoreParts(root);
    const delta=new THREE.Matrix4().makeRotationZ(Math.PI/4);delta.setPosition(0,.4,0);
    const ungrouped={group_inner:{...structure.group_inner,parent:'0',matrix:delta.toArray()}};
    applyParts(root,{group_inner:{x:.2,rz:30}},ungrouped);
    expect(modelPoint(root,joycon).distanceTo(expected)).toBeCloseTo(0);
    expect(partNode(scene,'group_outer')).toBeNull();
  });

  it('keeps exact affine matrices under nonuniform file parents and edits a grouped child',()=>{
    const {root,scene,console_,joycon,stick}=model();console_.scale.set(4,2,3);captureRestPose(root);
    applyParts(root,{'0_1':{x:.2,ry:15}});const before=modelPoint(root,stick);restoreParts(root);
    const structure={group_pair:{name:'Pair',parent:'0',members:['0_0','0_1'],pivot:[0,0,0]}};
    applyParts(root,{'0_1':{x:.2,ry:15},group_pair:{rz:45,scale:140}},structure);
    const expected=before.clone().applyAxisAngle(new THREE.Vector3(0,0,1),Math.PI/4).multiplyScalar(1.4);
    expect(modelPoint(root,stick).distanceTo(expected)).toBeCloseTo(0,7);
    const offset=partOffsetsForLocal(root,joycon,joycon.matrix.clone());expect(offset.x).toBeCloseTo(.2,7);expect(offset.ry).toBeCloseTo(15,7);
    expect(partNode(scene,'group_pair')!.userData.partBounds.isEmpty()).toBe(false);
  });

  it('rejects cyclic or multiply owned part groups',()=>{
    const part=(members:string[],parent:string|null=null)=>({name:'Group',members,parent,pivot:[0,0,0]});
    const parse=(parts:any)=>parseScene({objects:[{id:'m',source:{assetId:'a'},parts}]});
    expect(()=>parse({group_a:part(['group_b']),group_b:part(['group_a'])})).toThrow(/cycle/);
    expect(()=>parse({group_a:part(['0']),group_b:part(['0'])})).toThrow(/membership/);
    expect(()=>parse({group_a:part(['0'],'group_missing')})).toThrow(/parent/);
    expect(()=>parse({'0':part(['0_0'])})).toThrow(/group/);
  });
  it('lists named nodes as a tree with stable keys and original names', () => {
    const { root, scene, stick, primitive } = model();
    expect(partsRoot(root)).toBe(scene);
    expect(listParts(scene).map(p => [p.key, p.name, p.parent, p.depth])).toEqual([
      ['0', 'Console', null, 0], ['0_0', 'Screen', '0', 1], ['0_1', 'JoyCon R', '0', 1], ['0_1_0', 'Stick', '0_1', 2]
    ]);
    expect(partNode(scene, '0_1_0')).toBe(stick);
    expect(partKeyOf(scene, primitive)).toBe('0_1');
    expect(partKeyOf(scene, stick)).toBe('0_1_0');
  });

  it('moves a part along the model axes in model units, whatever its parents do', () => {
    const { root, joycon, stick } = model();
    const before = modelPoint(root, joycon), stickBefore = modelPoint(root, stick);
    applyParts(root, { '0_1': { x: .25 } });
    const after = modelPoint(root, joycon);
    expect(after.x - before.x).toBeCloseTo(.25); expect(after.y).toBeCloseTo(before.y); expect(after.z).toBeCloseTo(before.z);
    // Children ride along.
    expect(modelPoint(root, stick).x - stickBefore.x).toBeCloseTo(.25);
    // The next frame starts from the file pose again.
    restoreParts(root);
    expect(modelPoint(root, joycon).x).toBeCloseTo(before.x);
  });

  it('turns about the part origin around model axes, scales and hides', () => {
    const { root, joycon, stick } = model();
    const pivot = modelPoint(root, joycon), arm = modelPoint(root, stick).sub(pivot);
    applyParts(root, { '0_1': { rz: 90, scale: 200 }, '0_0': { visible: false } });
    expect(modelPoint(root, joycon).distanceTo(pivot)).toBeCloseTo(0);
    const turned = modelPoint(root, stick).sub(pivot);
    const expected = arm.clone().applyAxisAngle(new THREE.Vector3(0, 0, 1), Math.PI / 2).multiplyScalar(2);
    expect(turned.distanceTo(expected)).toBeCloseTo(0);
    expect(partNode(partsRoot(root), '0_0')!.visible).toBe(false);
  });

  it('turns a viewer gesture back into the same offsets', () => {
    const { root, joycon } = model();
    applyParts(root, { '0_1': { x: .1, y: -.2, z: .3, rx: 10, ry: 20, rz: 30, scale: 150 } });
    root.updateMatrixWorld(true);
    joycon.updateMatrix();
    const values = partOffsetsForLocal(root, joycon, joycon.matrix.clone());
    for (const [key, value] of Object.entries({ x: .1, y: -.2, z: .3, rx: 10, ry: 20, rz: 30, scale: 150 })) expect((values as any)[key]).toBeCloseTo(value, 6);
  });

  it('stores sparse part channels on a model and validates their paths', () => {
    const object = createObject('m', { assetId: 'a' });
    expect(object.parts).toEqual({});
    const scene = parseScene({ objects: [{ id: 'm', source: { assetId: 'a' }, parts: { '0_1': { name: 'JoyCon R', p: { x: .5 } } } }] });
    expect(scene.objects[0]!.parts['0_1']!.p.x).toEqual({ v: .5, kf: [], expr: null });
    expect(scene.objects[0]!.parts['0_1']!.p.scale.v).toBe(100);
    expect(() => parseScene({ objects: [{ id: 'm', source: { assetId: 'a' }, parts: { 'bad key': { p: {} } } }] })).toThrow();
    expect(() => validateSceneChannel('pt.0_1.x', { v: 1, kf: [], expr: null })).not.toThrow();
    expect(() => validateSceneChannel('pt.0_1.scale', { v: -5, kf: [], expr: null })).toThrow();
    const layer = { d: { definition: 'powermove.3d.object', data: { object: scene.objects[0] } } };
    const rows = layer3DProperties(layer).filter(row => row.key.startsWith('pt.'));
    expect(rows.map(row => [row.key, row.group])).toContainEqual(['pt.0_1.x', 'JoyCon R']);
  });
});

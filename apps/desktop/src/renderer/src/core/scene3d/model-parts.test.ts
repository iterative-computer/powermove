import { describe, expect, it } from 'vitest';
import * as THREE from 'three';
import { PARTS_ROOT_FLAG, applyParts, captureRestPose, listParts, partKeyOf, partNode, partOffsetsForLocal, partsRoot, restoreParts } from './model-parts';
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

/**
 * Time and space cases for the native renderer's motion lane
 * (docs/engine/motion.md): motion blur (layers, groups, precomps, video,
 * shutter settings, several sample counts), 2.5D (the 3D switch, Z,
 * anchor and scale with Z, X and Y rotation, orientation, perspective per
 * layer and per 3D group, homography planes, coplanar ordering, the
 * painter sort, planes crossing the camera plane), and retiming (eased and
 * expression speed at the root and nested, time remapping, and frame
 * selection on frame-counter video). Every layer is a type the native
 * renderer draws (solids, images, video, precomps, groups), so each case
 * isolates this lane's features.
 */
import { newProject, type Builder, type Doc } from '../doc';
import { ref } from '../media';
import { GRADIENT, project, type CorpusProject } from './common';

const DARK = { type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: '#14161C', position: 0 }] };
/** Today's export counts are captured anyway (16 opaque, 20 alpha). */
const BLUR_COUNTS = [1, 3, 6, 12];

function blurSolids(): CorpusProject {
  const b = newProject('motion/blur-solids', { name: 'Motion blur: solids and images', w: 480, h: 270, backgroundFill: GRADIENT });
  b.add(b.layer('solid', { name: 'Static back', d: { color: '#1F2433', w: 480, h: 70 }, p: { 'position.y': 200 } }));
  // Fast linear slide, rotation, and scale with their own anchors.
  b.add(b.layer('solid', { name: 'Slide (blur)', mblur: true, d: { color: '#FF6B1A', w: 60, h: 40 }, p: { 'position.x': b.ch(20, [[0, 20], [1, 420]]), 'position.y': 20 } }));
  b.add(b.layer('solid', { name: 'Slide (no blur)', d: { color: '#FF6B1A', w: 60, h: 40 }, p: { 'position.x': b.ch(20, [[0, 20], [1, 420]]), 'position.y': 70 } }));
  b.add(b.layer('solid', { name: 'Spin (blur, 80%)', mblur: true, d: { color: '#4C8DFF', w: 120, h: 18 }, p: { 'position.x': 120, 'position.y': 160, 'anchor.x': 60, 'anchor.y': 9, rotation: b.ch(0, [[0, 0], [2, 1080]]), opacity: 80 } }));
  b.add(b.layer('solid', { name: 'Zoom (blur, eased)', mblur: true, d: { color: '#3FCF8E', w: 50, h: 50 }, p: { 'position.x': 300, 'position.y': 160, 'anchor.x': 25, 'anchor.y': 25, 'scale.x': b.ch(20, [[0, 20, { legacy: 'power' }], [1.5, 260, { legacy: 'power' }]]), 'scale.y': b.ch(20, [[0, 20, { legacy: 'power' }], [1.5, 260, { legacy: 'power' }]]) } }));
  // A translucent color (#RRGGBBAA) and a skewed, parented mover.
  b.add(b.layer('solid', { name: 'Translucent color (blur)', mblur: true, d: { color: '#F2C14E99', w: 70, h: 70 }, p: { 'position.x': b.ch(400, [[0, 400], [2, 40]]), 'position.y': b.ch(120, [[0, 120], [2, 40]]), skew: 20 } }));
  const rig = b.add(b.layer('null', { name: 'Rig', p: { 'position.x': 380, 'position.y': 220, rotation: b.ch(0, [[0, 0], [2, -360]]) } }));
  b.add(b.layer('solid', { name: 'Parented arm (blur)', mblur: true, parent: rig.id, d: { color: '#E8E2CF', w: 70, h: 12 }, p: { 'position.x': 0, 'position.y': 0, 'anchor.y': 6 } }));
  // Images: rotated with bilinear edges, minified, alpha.
  b.add(b.layer('image', { name: 'Image (blur)', mblur: true, d: { asset: ref('still-alpha.png'), fit: 'contain', w: 160, h: 90 }, p: { 'position.x': b.ch(80, [[0, 80], [2, 400]]), 'position.y': 110, rotation: b.ch(-20, [[0, -20], [2, 160]]), 'scale.x': 70, 'scale.y': 70 } }));
  b.add(b.layer('image', { name: 'Photo (blur, slow)', mblur: true, d: { asset: ref('corpus-photo-lossless-opaque.webp'), fit: 'cover', w: 120, h: 72 }, p: { 'position.x': b.ch(400, [[0, 400], [2, 370]]), 'position.y': 60 } }));
  return project(b, { group: 'Motion', title: 'Motion blur on solids and images', covers: ['layer motion blur', 'sample counts', 'opacity divided among samples', 'eased motion', 'translucent solid color', 'parented motion', 'image motion blur', 'static layer with blur switch'], samples: [0.2, 0.5, 1, 1.6], alpha: true, blurSamples: BLUR_COUNTS });
}

function blurShutter(): CorpusProject[] {
  const wide = newProject('motion/shutter-wide', { name: 'Shutter 1.5 frames at 24 fps', w: 480, h: 270, fps: 24, shutter: 1.5, backgroundFill: DARK });
  wide.add(wide.layer('solid', { name: 'Slide (blur)', mblur: true, d: { color: '#FF6B1A', w: 40, h: 40 }, p: { 'position.x': wide.ch(0, [[0, 0], [2, 440]]), 'position.y': 40 } }));
  // Starts at 1 s: at t = 1 half of its samples fall before its start, and
  // are still drawn (today's samples do not re-check the active range).
  wide.add(wide.layer('solid', { name: 'Late start (blur)', mblur: true, from: 1, dur: 1, d: { color: '#4C8DFF', w: 40, h: 40 }, p: { 'position.x': wide.ch(0, [[0, 0], [1, 440]]), 'position.y': 120 } }));
  // Hold keys: a jump inside the shutter.
  wide.add(wide.layer('solid', { name: 'Hold jump (blur)', mblur: true, d: { color: '#3FCF8E', w: 40, h: 40 }, p: { 'position.x': wide.ch(40, [[0, 40, { hold: true }], [1, 400, { hold: true }]]), 'position.y': 200 } }));
  const zero = newProject('motion/shutter-zero', { name: 'Shutter 0 (today: half a frame)', w: 320, h: 180, shutter: 0, backgroundFill: DARK });
  zero.add(zero.layer('solid', { name: 'Slide (blur)', mblur: true, d: { color: '#F2C14E', w: 30, h: 30 }, p: { 'position.x': zero.ch(0, [[0, 0], [1, 300]]), 'position.y': 75 } }));
  return [
    project(wide, { group: 'Motion', title: 'Wide shutter at 24 fps', covers: ['composition shutter', '24 fps shutter division', 'samples outside the layer range', 'hold keyframes inside the shutter'], samples: [0.5, 1, 1.5], alpha: true, blurSamples: [6] }),
    project(zero, { group: 'Motion', title: 'Shutter 0', covers: ['shutter 0 is half a frame (`shutter || .5`)'], samples: [0.5], alpha: true }),
  ];
}

function blurGroups(): CorpusProject {
  const b = newProject('motion/blur-groups', { name: 'Motion blur on groups', w: 480, h: 270, backgroundFill: GRADIENT });
  const blurred = b.layer('group', { name: 'Group (blur, 70%)', mblur: true, p: { opacity: 70, 'position.x': b.ch(0, [[0, 0], [2, 220]]) } });
  b.add(b.layer('solid', { name: 'Member A', group: blurred.id, d: { color: '#FF6B1A', w: 80, h: 80 }, p: { 'position.x': 20, 'position.y': 20, rotation: b.ch(0, [[0, 0], [2, 90]]) } }));
  b.add(b.layer('solid', { name: 'Member B (own blur ignored)', mblur: true, group: blurred.id, d: { color: '#4C8DFF', w: 80, h: 80 }, p: { 'position.x': 60, 'position.y': 60 } }));
  const inner = b.layer('group', { name: 'Nested group (blur off inside)', mblur: true, group: blurred.id, p: { 'position.y': b.ch(0, [[0, 0], [2, 60]]) } });
  b.add(b.layer('solid', { name: 'Nested member', group: inner.id, d: { color: '#3FCF8E', w: 40, h: 40 }, p: { 'position.x': 150, 'position.y': 30 } }));
  b.add(inner);
  b.add(blurred);
  const plain = b.layer('group', { name: 'Group without blur', p: { 'position.y': 150 } });
  b.add(b.layer('solid', { name: 'Member with blur', mblur: true, group: plain.id, d: { color: '#F2C14E', w: 60, h: 30 }, p: { 'position.x': b.ch(20, [[0, 20], [1, 400]]), 'position.y': 20 } }));
  b.add(b.layer('solid', { name: 'Member without blur', group: plain.id, d: { color: '#A73366', w: 60, h: 30 }, p: { 'position.x': b.ch(20, [[0, 20], [1, 400]]), 'position.y': 70 } }));
  b.add(plain);
  return project(b, { group: 'Motion', title: 'Motion blur on groups', covers: ['group motion blur (members sampled together)', 'members inside a blurred group are not blurred again', 'group opacity over the accumulated samples', 'blurred member of a plain group'], samples: [0.3, 1, 1.7], alpha: true, blurSamples: [3, 6] });
}

function blurPrecomps(): CorpusProject {
  const b = newProject('motion/blur-precomps', { name: 'Motion blur with precomps', w: 480, h: 270, backgroundFill: DARK });
  const inner = b.composition({ name: 'Inner 24 fps', w: 200, h: 120, fps: 24, dur: 3, bg: '#000000' });
  b.within(inner, () => {
    b.add(b.layer('solid', { name: 'Inner back', d: { color: '#2A2F45', w: 200, h: 120 } }));
    b.add(b.layer('solid', { name: 'Inner spinner (blur)', mblur: true, d: { color: '#FF6B1A', w: 80, h: 14 }, p: { 'position.x': 100, 'position.y': 60, 'anchor.x': 40, 'anchor.y': 7, rotation: b.ch(0, [[0, 0], [3, 1440]]) } }));
    b.add(b.layer('solid', { name: 'Inner slider', d: { color: '#4C8DFF', w: 20, h: 20 }, p: { 'position.x': b.ch(0, [[0, 0], [3, 180]]), 'position.y': 90 } }));
  });
  // The precomp itself blurred: its content is sampled at the child times.
  b.add(b.layer('precomp', { name: 'Precomp (blur, moving)', mblur: true, d: { comp: inner.id, w: 200, h: 120 }, p: { 'position.x': b.ch(10, [[0, 10], [2, 260]]), 'position.y': 10 } }));
  // Not blurred itself: the nested layer blurs with the inner rate.
  b.add(b.layer('precomp', { name: 'Precomp (nested blur only)', d: { comp: inner.id, w: 200, h: 120 }, p: { 'position.x': 140, 'position.y': 140, rotation: 4 } }));
  // A retimed blurred precomp.
  b.add(b.layer('precomp', { name: 'Precomp fast (blur)', mblur: true, d: { comp: inner.id, w: 200, h: 120, speed: 2, trim: 0.25 }, p: { 'position.x': 20, 'position.y': 150, 'scale.x': 50, 'scale.y': 50 } }));
  return project(b, { group: 'Motion', title: 'Motion blur with precomps', covers: ['precomp motion blur', 'nested motion blur at the inner rate', 'nested blur inside a blurred precomp', 'retimed blurred precomp'], samples: [0.4, 1.1, 2], alpha: true, blurSamples: [3] });
}

function blurVideo(): CorpusProject {
  const b = newProject('motion/blur-video', { name: 'Motion blur on video', w: 480, h: 270, dur: 3, backgroundFill: DARK });
  b.add(b.layer('video', { name: 'Video (blur, moving)', mblur: true, d: { asset: ref('corpus-frames-30.mp4'), fit: 'contain', w: 160, h: 90 }, p: { 'position.x': b.ch(90, [[0, 90], [2, 390]]), 'position.y': 60 } }));
  b.add(b.layer('video', { name: 'Video (blur, static)', mblur: true, d: { asset: ref('corpus-frames-24.mp4'), fit: 'contain', w: 160, h: 90 }, p: { 'position.x': 120, 'position.y': 180 } }));
  b.add(b.layer('video', { name: 'Video (no blur)', d: { asset: ref('corpus-frames-24.mp4'), fit: 'contain', w: 160, h: 90 }, p: { 'position.x': 360, 'position.y': 180 } }));
  return project(b, { group: 'Motion', title: 'Motion blur on video', covers: ['video motion blur (the center frame in every sample)', 'static video with blur'], samples: [0.5, 1.2] });
}

function planes(): CorpusProject {
  const b = newProject('space25d/planes', { name: '2.5D planes', w: 640, h: 360, backgroundFill: GRADIENT });
  b.add(b.layer('solid', { name: 'Floor (rotation X, Z)', threeD: true, d: { color: '#2E3A55', w: 520, h: 300 }, p: { 'position.x': 60, 'position.y': 210, 'rotation.x': 72, 'position.z': 60 } }));
  b.add(b.layer('solid', { name: 'Swinging card (rotation Y, own lens)', threeD: true, d: { color: '#FF6B1A', w: 160, h: 110 }, p: { 'position.x': 60, 'position.y': 40, 'rotation.y': b.ch(-60, [[0, -60], [2.5, 60]]), perspective: 35, 'anchor.x': 80, 'anchor.y': 55 } }));
  b.add(b.layer('image', { name: 'Image in depth (Z, orientation, anchor Z, scale Z)', threeD: true, d: { asset: ref('still-alpha.png'), fit: 'contain', w: 200, h: 112 }, p: { 'position.x': 450, 'position.y': 120, 'position.z': b.ch(-200, [[0, -200], [2.5, 250]]), 'orientation.y': 25, 'orientation.x': -10, 'anchor.z': 40, 'scale.z': 150 } }));
  b.add(b.layer('solid', { name: 'Coplanar A (lower in timeline)', threeD: true, d: { color: '#4C8DFF', w: 100, h: 100 }, p: { 'position.x': 470, 'position.y': 230, 'rotation.y': 30 } }));
  b.add(b.layer('solid', { name: 'Coplanar B (upper)', threeD: true, d: { color: '#3FCF8E', w: 100, h: 100 }, p: { 'position.x': 500, 'position.y': 250, 'rotation.y': 30 } }));
  // The painter sort: C is above D in the timeline but passes behind it.
  b.add(b.layer('solid', { name: 'D (z 100 to -100)', threeD: true, d: { color: '#A73366', w: 120, h: 80 }, p: { 'position.x': 230, 'position.y': 230, 'position.z': b.ch(100, [[0, 100], [2.5, -100]]) } }));
  b.add(b.layer('solid', { name: 'C (z -100 to 100)', threeD: true, d: { color: '#F2C14E', w: 120, h: 80 }, p: { 'position.x': 260, 'position.y': 250, 'position.z': b.ch(-100, [[0, -100], [2.5, 100]]) } }));
  // X, Y, and Z rotation with skew and non-uniform scale.
  b.add(b.layer('solid', { name: 'Tumbling (all rotations, skew)', threeD: true, d: { color: '#7DD3FC', w: 90, h: 60 }, p: { 'position.x': 330, 'position.y': 90, 'anchor.x': 45, 'anchor.y': 30, rotation: b.ch(0, [[0, 0], [2.5, 90]]), 'rotation.x': b.ch(10, [[0, 10], [2.5, 140]]), 'rotation.y': 35, 'orientation.z': 15, skew: 12, 'scale.x': 120, 'scale.y': 80 } }));
  // A wide plane turned nearly edge-on with its far side behind the camera
  // plane (w ≤ 0 clipped by the rasterizer).
  b.add(b.layer('solid', { name: 'Through the camera plane', threeD: true, d: { color: '#E8E2CF', w: 900, h: 40 }, p: { 'position.x': -130, 'position.y': 300, 'anchor.x': 450, 'anchor.y': 20, 'rotation.y': 80, 'position.z': -820 } }));
  b.add(b.layer('solid', { name: '2D overlay (barrier)', d: { color: '#FFFFFF', w: 640, h: 4 }, p: { 'position.y': 8 } }));
  b.add(b.layer('solid', { name: '3D above the barrier', threeD: true, d: { color: '#FB7185', w: 60, h: 60 }, p: { 'position.x': 20, 'position.y': 280, 'rotation.y': -40 } }));
  return project(b, { group: 'Space', title: '2.5D planes', covers: ['3D switch', 'Z position', 'anchor Z and scale Z', 'X and Y rotation', 'orientation', 'perspective per layer', 'homography planes', 'coplanar ordering', 'painter sort', '2D barrier', 'planes behind the camera plane', 'skew on a 3D layer'], samples: [0, 0.8, 1.25, 2.5], alpha: true });
}

function spaceGroups(): CorpusProject {
  const b = newProject('space25d/groups', { name: '3D groups', w: 640, h: 360, backgroundFill: DARK });
  const rig = b.layer('group', { name: '3D rig (lens 60)', threeD: true, p: { 'position.x': 320, 'position.y': 180, 'anchor.x': 320, 'anchor.y': 180, 'rotation.y': b.ch(-40, [[0, -40], [2.5, 40]]), 'rotation.x': 15, 'position.z': 80, perspective: 60 } });
  const flat = b.layer('group', { name: 'Flat inner group', group: rig.id, p: { 'scale.x': 90, 'scale.y': 90, 'position.x': 320, 'position.y': 180, 'anchor.x': 320, 'anchor.y': 180 } });
  b.add(b.layer('solid', { name: 'Left card', group: flat.id, d: { color: '#FF6040', w: 130, h: 140 }, p: { 'position.x': 175, 'position.y': 110 } }));
  b.add(b.layer('solid', { name: 'Right card (own lens ignored)', threeD: true, group: flat.id, d: { color: '#4060FF', w: 130, h: 140 }, p: { 'position.x': 335, 'position.y': 110, 'position.z': -60, perspective: 20 } }));
  b.add(flat);
  b.add(b.layer('image', { name: 'Image in the rig', group: rig.id, d: { asset: ref('still-alpha.png'), fit: 'contain', w: 200, h: 112 }, p: { 'position.x': 320, 'position.y': 300 } }));
  b.add(rig);
  // A 3D group that is a compositing boundary (opacity): its members are
  // planes inside its own target.
  const faded = b.layer('group', { name: '3D group at 60%', threeD: true, p: { opacity: 60, 'rotation.x': b.ch(-30, [[0, -30], [2.5, 30]]), 'position.x': 520, 'position.y': 60, 'anchor.x': 520, 'anchor.y': 60 } });
  b.add(b.layer('solid', { name: 'Faded A', group: faded.id, d: { color: '#3FCF8E', w: 90, h: 60 }, p: { 'position.x': 460, 'position.y': 20 } }));
  b.add(b.layer('solid', { name: 'Faded B', group: faded.id, d: { color: '#F2C14E', w: 90, h: 60 }, p: { 'position.x': 500, 'position.y': 50, 'position.z': -40 } }));
  b.add(faded);
  return project(b, { group: 'Space', title: '3D groups with flat children', covers: ['perspective per 3D group', 'nested flat groups in a 3D group', 'member lens ignored inside a 3D group', '3D group compositing boundary'], samples: [0, 1.25, 2.5], alpha: true });
}

function spacePrecomps(): CorpusProject {
  const b = newProject('space25d/precomps', { name: '2.5D precomps and video', w: 640, h: 360, dur: 3, backgroundFill: GRADIENT });
  const inner = b.composition({ name: 'Inner 320x200', w: 320, h: 200, dur: 3, bg: '#000000' });
  b.within(inner, () => {
    b.add(b.layer('solid', { name: 'Inner back', d: { color: '#202638', w: 320, h: 200 } }));
    // 3D inside a smaller composition: its own size centers the lens.
    b.add(b.layer('solid', { name: 'Inner tilted', threeD: true, d: { color: '#FF6B1A', w: 160, h: 100 }, p: { 'position.x': 80, 'position.y': 50, 'rotation.y': b.ch(50, [[0, 50], [3, -50]]), 'anchor.x': 80, 'anchor.y': 50 } }));
  });
  b.add(b.layer('precomp', { name: 'Precomp as a plane', threeD: true, d: { comp: inner.id, w: 320, h: 200 }, p: { 'position.x': 20, 'position.y': 20, 'rotation.x': b.ch(-35, [[0, -35], [3, 35]]), 'anchor.x': 160, 'anchor.y': 100, 'position.z': 40 } }));
  b.add(b.layer('video', { name: 'Video plane', threeD: true, d: { asset: ref('corpus-frames-30.mp4'), fit: 'contain', w: 240, h: 135 }, p: { 'position.x': 470, 'position.y': 250, 'rotation.y': b.ch(-50, [[0, -50], [3, 50]]), 'position.z': -80 } }));
  return project(b, { group: 'Space', title: '2.5D precomps and video', covers: ['3D precomp layer', '3D layers inside a precomp (its own size)', '3D video layer'], samples: [0.4, 1.5, 2.6], alpha: true });
}

function spaceBlur(): CorpusProject {
  const b = newProject('space25d/blur', { name: '2.5D with motion blur', w: 480, h: 270, backgroundFill: DARK });
  b.add(b.layer('solid', { name: 'Spinning plane (blur)', threeD: true, mblur: true, d: { color: '#4C8DFF', w: 160, h: 100 }, p: { 'position.x': 160, 'position.y': 135, 'anchor.x': 80, 'anchor.y': 50, 'rotation.y': b.ch(0, [[0, 0], [2, 540]]) } }));
  b.add(b.layer('solid', { name: 'Approaching plane (blur)', threeD: true, mblur: true, d: { color: '#FF6B1A', w: 80, h: 80 }, p: { 'position.x': 330, 'position.y': 100, 'position.z': b.ch(600, [[0, 600], [2, -400]]) } }));
  return project(b, { group: 'Space', title: '2.5D with motion blur', covers: ['motion blur on 2.5D planes'], samples: [0.5, 1.3], alpha: true, blurSamples: [6] });
}

function speedVideo(b: Builder, name: string, x: number, y: number, speed: Doc, asset = 'corpus-frames-30.mp4'): void {
  b.add(b.layer('video', { name, d: { asset: ref(asset), fit: 'contain', w: 160, h: 90, speed }, p: { 'position.x': x, 'position.y': y } }));
}

function retiming(): CorpusProject[] {
  const root = newProject('retime/speed', { name: 'Variable speed at the root', w: 480, h: 270, dur: 3, backgroundFill: DARK });
  // Eased speed: the midpoint rule does not integrate it exactly, so the
  // step size shows (amendment 16: the root matches today exactly).
  speedVideo(root, 'Eased speed', 80, 45, root.ch(0.5, [[0, 0.5, { legacy: 'power' }], [2.5, 1.8, { legacy: 'power' }]]));
  speedVideo(root, 'Expression speed', 240, 45, root.ch(1, [], 'value * (1 + 0.4 * Math.sin(time * 5))'));
  speedVideo(root, 'Constant 1.5x, trim', 400, 45, root.ch(1.5));
  speedVideo(root, '24 fps source at 0.8x', 80, 135, root.ch(0.8), 'corpus-frames-24.mp4');
  root.add(root.layer('video', { name: 'Remap ease', d: { asset: ref('corpus-frames-30.mp4'), fit: 'contain', w: 160, h: 90, timeRemap: root.ch(true), sourceTime: root.ch(0, [[0, 0, { legacy: 'power' }], [2.5, 3.5, { legacy: 'power' }]]) }, p: { 'position.x': 240, 'position.y': 135 } }));
  root.add(root.layer('video', { name: 'Remap freeze', d: { asset: ref('corpus-frames-24.mp4'), fit: 'contain', w: 160, h: 90, timeRemap: root.ch(true), sourceTime: root.ch(1.37, [[0, 1.37, { hold: true }]]) }, p: { 'position.x': 400, 'position.y': 135 } }));
  // Precomps: eased speed, and nested compositions at the root's rate
  // (identical answers) and at 24 fps (amendment 16's allowed difference).
  const same = root.composition({ name: 'Nested 30 fps', w: 160, h: 90, fps: 30, dur: 3, bg: '#000000' });
  root.within(same, () => speedVideo(root, 'Nested eased speed (30)', 80, 45, root.ch(0.6, [[0, 0.6, { legacy: 'power' }], [2.5, 1.7, { legacy: 'power' }]])));
  const other = root.composition({ name: 'Nested 24 fps', w: 160, h: 90, fps: 24, dur: 3, bg: '#000000' });
  root.within(other, () => speedVideo(root, 'Nested eased speed (24)', 80, 45, root.ch(0.6, [[0, 0.6, { legacy: 'power' }], [2.5, 1.7, { legacy: 'power' }]])));
  root.add(root.layer('precomp', { name: 'Nested at the root rate', d: { comp: same.id, w: 160, h: 90 }, p: { 'position.x': 0, 'position.y': 180 } }));
  root.add(root.layer('precomp', { name: 'Nested at 24 fps', d: { comp: other.id, w: 160, h: 90 }, p: { 'position.x': 160, 'position.y': 180 } }));
  root.add(root.layer('precomp', { name: 'Precomp eased speed', d: { comp: same.id, w: 160, h: 90, speed: root.ch(0.4, [[0, 0.4, { legacy: 'power' }], [2.5, 2, { legacy: 'power' }]]) }, p: { 'position.x': 320, 'position.y': 180 } }));
  return [
    project(root, { group: 'Retiming', title: 'Variable speed, remaps, and frame selection', covers: ['eased speed at the root', 'expression speed', 'constant speed and trim', '24 fps source in a 30 fps composition', 'eased time remap', 'freeze', 'nested variable speed at the root rate', 'nested variable speed at its own rate (amendment 16)', 'precomp eased speed'], samples: [0.4, 0.93, 1.5, 2.2, 2.9] }),
  ];
}

export function motionProjects(): CorpusProject[] {
  return [blurSolids(), ...blurShutter(), blurGroups(), blurPrecomps(), blurVideo(), planes(), spaceGroups(), spacePrecomps(), spaceBlur(), ...retiming()];
}

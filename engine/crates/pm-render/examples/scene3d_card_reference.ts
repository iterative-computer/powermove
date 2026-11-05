// Independent 3D Library card light rendered by Three.js r180 in RGBA32F.
// Bundle this browser entry outside pm-electron-lock before the E2E test.
import * as THREE from '../../../../apps/desktop/node_modules/three/build/three.module.js';

(globalThis as any).cardFloatReference = ({ width, height, view }: {
  width: number; height: number; view: any;
}) => {
  const k = 8, sw = width * k, sh = height * k;
  const scene = new THREE.Scene();
  const ambient = new THREE.HemisphereLight(0xffffff, 0xffffff, .5);
  ambient.groundColor.multiplyScalar(.35); scene.add(ambient);
  const light = new THREE.PointLight(0xffffff, 30, 0, 2);
  light.position.set(-2, 3, 2); light.castShadow = true;
  light.shadow.mapSize.set(1024, 1024); light.shadow.bias = -.0002; light.shadow.normalBias = .025;
  scene.add(light);
  const cube = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshStandardMaterial({
    color: '#ff7733', roughness: .4, metalness: .1, side: THREE.DoubleSide,
  }));
  cube.castShadow = true; cube.receiveShadow = true; scene.add(cube);
  const spec = view?.camera;
  let camera: THREE.Camera;
  if (spec?.projection.kind === 'orthographic') {
    const p = spec.projection, halfWidth = p.halfHeight * width / height;
    camera = new THREE.OrthographicCamera(-halfWidth, halfWidth, p.halfHeight, -p.halfHeight, p.near, p.far);
  } else {
    const p = spec?.projection;
    camera = new THREE.PerspectiveCamera(p?.fovYDegrees ?? 45, width / height, p?.near ?? .01, p?.far ?? 1000);
  }
  if (spec) {
    const world = new THREE.Matrix4().fromArray(spec.view).invert();
    world.decompose(camera.position, camera.quaternion, camera.scale);
  } else {
    camera.position.set(0, 1.5, 7); camera.lookAt(0, 0, 0);
  }
  (camera as THREE.PerspectiveCamera | THREE.OrthographicCamera).updateProjectionMatrix();
  camera.updateMatrixWorld(true);
  const renderer = new THREE.WebGLRenderer({ alpha: true, antialias: false, premultipliedAlpha: true });
  renderer.outputColorSpace = THREE.LinearSRGBColorSpace;
  renderer.toneMapping = THREE.NoToneMapping;
  renderer.shadowMap.enabled = true; renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  renderer.setPixelRatio(1); renderer.setSize(sw, sh, false); renderer.setClearColor(0, 0);
  const gl = renderer.getContext();
  if (!gl.getExtension('EXT_color_buffer_float')) throw new Error('float color attachment unavailable for 3D reference');
  const target = new THREE.WebGLRenderTarget(sw, sh, { type: THREE.FloatType, format: THREE.RGBAFormat,
    colorSpace: THREE.LinearSRGBColorSpace, depthBuffer: true, samples: 0 });
  const floats = new Float32Array(sw * sh * 4);
  try {
    renderer.setRenderTarget(target); renderer.clear(); renderer.render(scene, camera);
    renderer.readRenderTargetPixels(target, 0, 0, sw, sh, floats);
  } finally { target.dispose(); cube.geometry.dispose(); (cube.material as THREE.Material).dispose(); renderer.dispose(); }
  return floats;
};

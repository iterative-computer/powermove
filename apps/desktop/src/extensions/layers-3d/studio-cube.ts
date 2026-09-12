import type { ExtensionLayerDefinition } from 'powermove';

export const STUDIO_CUBE_ID = 'powermove.3d.studio-cube';

export const STUDIO_CUBE: ExtensionLayerDefinition = {
  id: STUDIO_CUBE_ID,
  label: '3D Studio Cube',
  version: 1,
  icon: 'layers',
  color: '#9B8CFF',
  params: [
    { k: 'yaw', label: 'Camera yaw', def: 28, min: -180, max: 180, step: 1, unit: '°' },
    { k: 'pitch', label: 'Camera pitch', def: 18, min: -70, max: 70, step: 1, unit: '°' },
    { k: 'distance', label: 'Camera distance', def: 4.8, min: 2.5, max: 10, step: 0.05 },
    { k: 'rotation', label: 'Cube rotation', def: 25, min: -360, max: 360, step: 1, unit: '°' },
    { k: 'tilt', label: 'Cube tilt', def: -12, min: -180, max: 180, step: 1, unit: '°' },
    { k: 'size', label: 'Cube size', def: 1.25, min: 0.35, max: 2.2, step: 0.01 },
    { k: 'roundness', label: 'Edge roundness', def: 0.12, min: 0, max: 0.45, step: 0.005 },
    { k: 'roughness', label: 'Roughness', def: 0.2, min: 0.02, max: 1, step: 0.01 },
    { k: 'metalness', label: 'Metalness', def: 0.92, min: 0, max: 1, step: 0.01 },
    { k: 'objectColor', label: 'Object color', def: '#C7C4FF', type: 'color' },
    { k: 'lightColor', label: 'Key light', def: '#FFB36B', type: 'color' },
    { k: 'background', label: 'Background', def: '#0C0D12', type: 'color' },
    { k: 'autoRotate', label: 'Auto rotate', def: true, type: 'toggle' },
    { k: 'speed', label: 'Rotation speed', def: 28, min: -180, max: 180, step: 1, unit: '°/s' }
  ],
  defaults: {
    objects: [{ id: 'cube', geometry: 'rounded-box', material: 'metal' }],
    camera: { id: 'camera', projection: 'perspective' },
    lights: [{ id: 'key', type: 'area' }, { id: 'rim', type: 'area' }]
  },
  renderer: {
    kind: 'fragment',
    fragment: `
float sdRoundBox3(vec3 p, vec3 b, float r) {
  vec3 q = abs(p) - b + r;
  return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0) - r;
}

mat3 rotateX(float a) {
  float c = cos(a), s = sin(a);
  return mat3(1,0,0, 0,c,s, 0,-s,c);
}

mat3 rotateY(float a) {
  float c = cos(a), s = sin(a);
  return mat3(c,0,-s, 0,1,0, s,0,c);
}

vec2 sceneDistance(vec3 p) {
  float spin = radians(u_rotation + (u_autoRotate ? iTime * u_speed : 0.0));
  vec3 local = rotateX(radians(u_tilt)) * rotateY(spin) * p;
  float box = sdRoundBox3(local, vec3(u_size * 0.72), min(u_roundness, u_size * 0.45));
  float floorPlane = p.y + u_size * 0.95;
  return box < floorPlane ? vec2(box, 1.0) : vec2(floorPlane, 2.0);
}

vec3 sceneNormal(vec3 p) {
  vec2 e = vec2(0.0015, 0.0);
  return normalize(vec3(
    sceneDistance(p + e.xyy).x - sceneDistance(p - e.xyy).x,
    sceneDistance(p + e.yxy).x - sceneDistance(p - e.yxy).x,
    sceneDistance(p + e.yyx).x - sceneDistance(p - e.yyx).x
  ));
}

float softShadow(vec3 ro, vec3 rd) {
  float result = 1.0, t = 0.03;
  for (int i = 0; i < 40; i++) {
    float h = sceneDistance(ro + rd * t).x;
    result = min(result, 14.0 * h / t);
    t += clamp(h, 0.02, 0.18);
    if (h < 0.001 || t > 12.0) break;
  }
  return clamp(result, 0.0, 1.0);
}

void main() {
  vec2 p = uv * 2.0 - 1.0;
  p.y = -p.y;
  p.x *= iResolution.x / max(iResolution.y, 1.0);

  float yaw = radians(u_yaw), pitch = radians(u_pitch);
  vec3 ro = u_distance * vec3(cos(pitch) * sin(yaw), sin(pitch), cos(pitch) * cos(yaw));
  vec3 target = vec3(0.0, -0.08, 0.0);
  vec3 forward = normalize(target - ro);
  vec3 right = normalize(cross(forward, vec3(0.0, 1.0, 0.0)));
  vec3 up = cross(right, forward);
  vec3 rd = normalize(forward * 1.8 + right * p.x + up * p.y);

  float t = 0.0, material = 0.0;
  for (int i = 0; i < 96; i++) {
    vec2 hit = sceneDistance(ro + rd * t);
    material = hit.y;
    if (hit.x < 0.001 || t > 18.0) break;
    t += hit.x * 0.78;
  }

  vec3 bg = u_background;
  vec3 color = bg * (0.72 + 0.28 * (1.0 - uv.y));
  if (t <= 18.0) {
    vec3 pos = ro + rd * t;
    vec3 normal = sceneNormal(pos);
    vec3 view = normalize(ro - pos);
    vec3 keyDir = normalize(vec3(-3.5, 5.0, 4.0) - pos);
    vec3 rimDir = normalize(vec3(4.0, 2.5, -3.0) - pos);
    float key = max(dot(normal, keyDir), 0.0) * softShadow(pos + normal * 0.01, keyDir);
    float rim = pow(max(dot(normal, rimDir), 0.0), 1.5);
    float fresnel = pow(1.0 - max(dot(normal, view), 0.0), 4.0);
    vec3 halfVector = normalize(keyDir + view);
    float specPower = mix(18.0, 180.0, 1.0 - u_roughness);
    float spec = pow(max(dot(normal, halfVector), 0.0), specPower) * (0.2 + 2.5 * u_metalness);

    if (material < 1.5) {
      vec3 diffuse = u_objectColor * (0.12 + key * (1.0 - 0.7 * u_metalness));
      vec3 reflections = mix(u_objectColor, u_lightColor, 0.45) * (fresnel * (0.35 + u_metalness));
      color = diffuse + u_lightColor * spec + reflections + vec3(0.35, 0.42, 0.65) * rim * 0.35;
    } else {
      float grid = smoothstep(0.97, 1.0, max(abs(sin(pos.x * 3.14159)), abs(sin(pos.z * 3.14159))));
      color = mix(bg * 1.4, bg * 2.0, grid * 0.15) + u_lightColor * key * 0.035;
    }
  }
  color = color / (color + vec3(1.0));
  color = pow(color, vec3(1.0 / 2.2));
  fragColor = vec4(color, 1.0);
}`
  }
};

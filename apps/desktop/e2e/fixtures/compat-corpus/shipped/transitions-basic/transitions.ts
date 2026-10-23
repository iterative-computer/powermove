import type { TransitionDefinition } from 'powermove';

export const BUILTIN_TRANSITIONS: TransitionDefinition[] = [
  {
    id: 'crossfade', label: 'Crossfade', group: 'Basic', params: [],
    frag: 'o = mix(texture(u_from, v_st), texture(u_to, v_st), clamp(u_prog, 0.0, 1.0));',
  },
  {
    id: 'wipe', label: 'Wipe', group: 'Basic',
    params: [
      { k: 'angle', label: 'Angle', def: 0, min: -360, max: 360, step: 1, unit: '°' },
      { k: 'softness', label: 'Softness', def: 8, min: 0, max: 100, step: 1, unit: '%' },
    ],
    frag: `
      float a = radians(u_angle);
      vec2 dir = vec2(cos(a), sin(a));
      float span = max(abs(dir.x) + abs(dir.y), 0.0001);
      float pos = dot(v_st - 0.5, dir) / span + 0.5;
      float feather = max(u_softness / 200.0, 0.0001);
      float edge = mix(-feather, 1.0 + feather, clamp(u_prog, 0.0, 1.0));
      float reveal = 1.0 - smoothstep(edge - feather, edge + feather, pos);
      o = mix(texture(u_from, v_st), texture(u_to, v_st), reveal);`,
  },
  {
    id: 'slide', label: 'Slide', group: 'Movement',
    params: [
      { k: 'direction', label: 'Direction', def: 0, min: 0, max: 360, step: 1, unit: '°' },
    ],
    frag: `
      float p = clamp(u_prog, 0.0, 1.0);
      float a = radians(u_direction);
      vec2 dir = vec2(cos(a), -sin(a));
      vec2 fromUV = v_st - dir * p;
      vec2 toUV = v_st + dir * (1.0 - p);
      float fromInside = step(0.0, fromUV.x) * step(fromUV.x, 1.0) * step(0.0, fromUV.y) * step(fromUV.y, 1.0);
      float toInside = step(0.0, toUV.x) * step(toUV.x, 1.0) * step(0.0, toUV.y) * step(toUV.y, 1.0);
      vec4 fromColor = texture(u_from, clamp(fromUV, 0.0, 1.0)) * fromInside;
      vec4 toColor = texture(u_to, clamp(toUV, 0.0, 1.0)) * toInside;
      o = toColor + fromColor * (1.0 - toColor.a);`,
  },
  {
    id: 'zoom', label: 'Zoom', group: 'Movement',
    params: [
      { k: 'amount', label: 'Amount', def: 25, min: 0, max: 200, step: 1, unit: '%' },
    ],
    frag: `
      float p = clamp(u_prog, 0.0, 1.0);
      float amount = u_amount / 100.0;
      vec2 fromUV = 0.5 + (v_st - 0.5) * (1.0 - p * amount);
      vec2 toUV = 0.5 + (v_st - 0.5) * (1.0 + (1.0 - p) * amount);
      o = mix(texture(u_from, fromUV), texture(u_to, toUV), p);`,
  },
];

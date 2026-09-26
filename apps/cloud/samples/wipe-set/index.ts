import type { PowermoveAPI } from 'powermove';

export default function activate(api: PowermoveAPI) {
  api.transitions.register({
    id: `${api.id}.left`, label: 'Wipe left', params: [],
    frag: `o = mix(texture(u_from, v_st), texture(u_to, v_st), step(v_st.x, u_prog));`,
  });
  api.transitions.register({
    id: `${api.id}.right`, label: 'Wipe right', params: [],
    frag: `o = mix(texture(u_from, v_st), texture(u_to, v_st), step(1.0 - v_st.x, u_prog));`,
  });
  api.transitions.register({
    id: `${api.id}.iris`, label: 'Iris', params: [],
    frag: `float radius = length(v_st - vec2(0.5));
      o = mix(texture(u_from, v_st), texture(u_to, v_st), step(radius, u_prog * 0.72));`,
  });
}

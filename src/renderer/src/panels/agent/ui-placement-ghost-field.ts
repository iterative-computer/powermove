const VERTEX_SHADER = `#version 300 es
in vec2 a_position;
out vec2 v_uv;

void main() {
  v_uv = a_position * 0.5 + 0.5;
  gl_Position = vec4(a_position, 0.0, 1.0);
}`;

const FRAGMENT_SHADER = `#version 300 es
precision highp float;

in vec2 v_uv;
uniform vec2 u_resolution;
uniform float u_time;
out vec4 out_color;

float rounded_box(vec2 point, vec2 half_size, float radius) {
  vec2 q = abs(point) - half_size + radius;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

void main() {
  vec2 point = v_uv * u_resolution - u_resolution * 0.5;
  float radius = min(10.0, min(u_resolution.x, u_resolution.y) * 0.18);
  float distance_to_edge = rounded_box(point, u_resolution * 0.5 - 1.0, radius);

  vec2 direction = point / max(u_resolution, vec2(1.0));
  float around = atan(direction.y, direction.x);
  float travel = around - u_time * 0.22;
  float lead = pow(0.5 + 0.5 * cos(travel), 7.0);
  float trail = pow(0.5 + 0.5 * cos(travel + 1.7), 12.0) * 0.45;
  float field = lead + trail;

  float hairline = exp(-abs(distance_to_edge) * 1.7);
  float inner_glow = exp(-max(-distance_to_edge, 0.0) / 23.0)
    * (1.0 - smoothstep(-1.0, 2.0, distance_to_edge));
  float outer_glow = exp(-max(distance_to_edge, 0.0) / 5.0)
    * (1.0 - smoothstep(8.0, 16.0, distance_to_edge));

  vec3 cool = vec3(0.50, 0.66, 0.69);
  vec3 lilac = vec3(0.63, 0.58, 0.70);
  vec3 color = mix(cool, lilac, 0.5 + 0.5 * cos(travel * 0.65));
  float alpha = hairline * (0.055 + field * 0.19)
    + inner_glow * field * 0.038
    + outer_glow * field * 0.035;

  out_color = vec4(color * alpha, alpha);
}`;

const MAX_DEVICE_PIXEL_RATIO = 1.5;

type GhostRenderer = {
  draw(time: number): void;
  resize(): void;
  destroy(): void;
};

function compileShader(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader {
  const shader = gl.createShader(type);
  if (!shader) throw new Error('Unable to create ghost shader');
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const detail = gl.getShaderInfoLog(shader) || 'Unknown shader compile error';
    gl.deleteShader(shader);
    throw new Error(detail);
  }
  return shader;
}

function createRenderer(canvas: HTMLCanvasElement): GhostRenderer | null {
  const gl = canvas.getContext('webgl2', {
    alpha: true,
    antialias: false,
    depth: false,
    powerPreference: 'low-power',
    premultipliedAlpha: true,
    preserveDrawingBuffer: false
  });
  if (!gl) return null;

  const vertex = compileShader(gl, gl.VERTEX_SHADER, VERTEX_SHADER);
  let fragment: WebGLShader;
  try {
    fragment = compileShader(gl, gl.FRAGMENT_SHADER, FRAGMENT_SHADER);
  } catch (error) {
    gl.deleteShader(vertex);
    throw error;
  }
  const program = gl.createProgram();
  if (!program) {
    gl.deleteShader(vertex);
    gl.deleteShader(fragment);
    throw new Error('Unable to create ghost shader program');
  }
  gl.attachShader(program, vertex);
  gl.attachShader(program, fragment);
  gl.linkProgram(program);
  gl.deleteShader(vertex);
  gl.deleteShader(fragment);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    const detail = gl.getProgramInfoLog(program) || 'Unknown shader link error';
    gl.deleteProgram(program);
    throw new Error(detail);
  }

  const buffer = gl.createBuffer();
  if (!buffer) {
    gl.deleteProgram(program);
    throw new Error('Unable to create ghost geometry');
  }
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, -1, 1, 1, -1, 1, 1]), gl.STATIC_DRAW);
  const position = gl.getAttribLocation(program, 'a_position');
  const resolution = gl.getUniformLocation(program, 'u_resolution');
  const time = gl.getUniformLocation(program, 'u_time');
  if (position < 0 || !resolution || !time) {
    gl.deleteBuffer(buffer);
    gl.deleteProgram(program);
    throw new Error('Unable to bind ghost shader inputs');
  }
  gl.enableVertexAttribArray(position);
  gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
  gl.clearColor(0, 0, 0, 0);

  const resize = () => {
    const bounds = canvas.getBoundingClientRect();
    const dpr = Math.min(window.devicePixelRatio || 1, MAX_DEVICE_PIXEL_RATIO);
    const width = Math.max(1, Math.round(bounds.width * dpr));
    const height = Math.max(1, Math.round(bounds.height * dpr));
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }
    gl.viewport(0, 0, width, height);
  };

  return {
    resize,
    draw(now) {
      resize();
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.useProgram(program);
      gl.uniform2f(resolution, canvas.width, canvas.height);
      gl.uniform1f(time, now * 0.001);
      gl.drawArrays(gl.TRIANGLES, 0, 6);
    },
    destroy() {
      gl.deleteBuffer(buffer);
      gl.deleteProgram(program);
    }
  };
}

export function mountGhostEdgeField(canvas: HTMLCanvasElement): () => void {
  const motionQuery = window.matchMedia?.('(prefers-reduced-motion: reduce)');
  let renderer: GhostRenderer | null = null;
  let frame = 0;
  let intersecting = true;
  let destroyed = false;

  const stop = () => {
    if (!frame) return;
    cancelAnimationFrame(frame);
    frame = 0;
  };
  const canAnimate = () => !destroyed && !document.hidden && intersecting && !motionQuery?.matches && !!renderer;
  const draw = (now: number) => {
    frame = 0;
    if (!canAnimate()) return;
    renderer?.draw(now);
    frame = requestAnimationFrame(draw);
  };
  const sync = () => {
    stop();
    if (motionQuery?.matches) {
      canvas.dataset.ghostRenderer = 'static';
      canvas.dataset.ghostFallback = 'reduced-motion';
      return;
    }
    if (!renderer) {
      try {
        renderer = createRenderer(canvas);
      } catch {
        renderer = null;
      }
      if (!renderer) {
        canvas.dataset.ghostRenderer = 'static';
        canvas.dataset.ghostFallback = 'webgl-unavailable';
        return;
      }
      canvas.dataset.ghostRenderer = 'webgl';
      delete canvas.dataset.ghostFallback;
    }
    canvas.dataset.ghostRenderer = 'webgl';
    delete canvas.dataset.ghostFallback;
    renderer.resize();
    if (canAnimate()) frame = requestAnimationFrame(draw);
  };

  const resize = new ResizeObserver(() => renderer?.resize());
  resize.observe(canvas);
  const intersection = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver(entries => {
    intersecting = entries.some(entry => entry.target === canvas && entry.isIntersecting);
    sync();
  });
  intersection?.observe(canvas);
  const onContextLost = (event: Event) => {
    event.preventDefault();
    stop();
    renderer = null;
    canvas.dataset.ghostRenderer = 'static';
    canvas.dataset.ghostFallback = 'context-lost';
  };
  const onContextRestored = () => sync();
  canvas.addEventListener('webglcontextlost', onContextLost);
  canvas.addEventListener('webglcontextrestored', onContextRestored);
  document.addEventListener('visibilitychange', sync);
  motionQuery?.addEventListener?.('change', sync);
  sync();

  return () => {
    destroyed = true;
    stop();
    resize.disconnect();
    intersection?.disconnect();
    document.removeEventListener('visibilitychange', sync);
    motionQuery?.removeEventListener?.('change', sync);
    canvas.removeEventListener('webglcontextlost', onContextLost);
    canvas.removeEventListener('webglcontextrestored', onContextRestored);
    renderer?.destroy();
    renderer = null;
  };
}

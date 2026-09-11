/** A quiet orange scan over the actual exported frames; never part of the output. */
export function animateExportPreview(canvas: HTMLCanvasElement): () => void {
  if (window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) return () => {};
  const gl = canvas.getContext('webgl', { alpha: true, premultipliedAlpha: false });
  if (!gl) return () => {};
  const shaders: WebGLShader[] = [];
  const shader = (type: number, source: string) => {
    const s = gl.createShader(type)!; shaders.push(s); gl.shaderSource(s, source); gl.compileShader(s); return s;
  };
  const program = gl.createProgram()!;
  gl.attachShader(program, shader(gl.VERTEX_SHADER, 'attribute vec2 p; varying vec2 uv; void main(){uv=p*.5+.5;gl_Position=vec4(p,0.,1.);}'));
  gl.attachShader(program, shader(gl.FRAGMENT_SHADER, `precision mediump float;
    varying vec2 uv; uniform float time;
    void main(){
      float wave=fract(time*.18); float d=abs(uv.x-wave);
      float sweep=exp(-d*d*180.);
      float grain=fract(sin(dot(floor(uv*vec2(640.,360.)),vec2(12.9898,78.233))+floor(time*12.))*43758.5453);
      gl_FragColor=vec4(1.,.39,.08,sweep*(.045+.1*grain));
    }`));
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) { shaders.forEach(s => gl.deleteShader(s)); gl.deleteProgram(program); return () => {}; }
  gl.useProgram(program);
  const buffer = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1,1,-1,-1,1,-1,1,1,-1,1,1]), gl.STATIC_DRAW);
  const position = gl.getAttribLocation(program, 'p'); gl.enableVertexAttribArray(position); gl.vertexAttribPointer(position,2,gl.FLOAT,false,0,0);
  const time = gl.getUniformLocation(program,'time'); let frame = 0;
  canvas.width=640; canvas.height=360; gl.viewport(0,0,640,360);
  const draw = (now: number) => { gl.uniform1f(time,now/1000); gl.drawArrays(gl.TRIANGLES,0,6); frame=requestAnimationFrame(draw); };
  frame=requestAnimationFrame(draw);
  return () => { cancelAnimationFrame(frame); gl.deleteBuffer(buffer); shaders.forEach(s=>gl.deleteShader(s)); gl.deleteProgram(program); gl.getExtension('WEBGL_lose_context')?.loseContext(); };
}

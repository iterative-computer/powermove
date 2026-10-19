/** The runner's async PBO path must preserve every pixel that its blocking
 * path reads, including concurrent frames, resizes, and a sync snapshot
 * between submission and GPU completion. No screenshot leaves the app. */
import { build } from 'esbuild';
import path from 'node:path';
import { statfs } from 'node:fs/promises';
import { expect, launchApp, test } from './helpers/app';

test('fallback PBO frames are bit exact across concurrent draws and synchronous snapshots', async () => {
  test.setTimeout(120_000);
  const bundle = await build({
    stdin: { contents: `export { ShaderFallbackRunner } from './src/renderer/src/engine-connection/shader-fallback';`, resolveDir: path.resolve(__dirname, '..') },
    bundle: true, write: false, format: 'iife', globalName: 'FallbackTest', target: 'es2022',
  });
  const disk = await statfs('/System/Volumes/Data');
  if (disk.bavail * disk.bsize < 40 * 1024 ** 3) throw new Error('Disk below required 40 GiB before app launch');
  const app = await launchApp();
  try {
    await app.page.evaluate(bundle.outputFiles[0].text);
    const result = await app.page.evaluate(async () => {
      const Runner = (window as any).FallbackTest.ShaderFallbackRunner;
      const runner = new Runner();
      const vertex = '#version 300 es\nin vec2 a_pos; void main(){gl_Position=vec4(a_pos*2.-1.,0,1);}';
      const fragment = '#version 300 es\nprecision highp float; uniform float t; out vec4 color; void main(){color=vec4(sin(gl_FragCoord.x*.13+t), gl_FragCoord.y*.01, t, .75);}';
      const request = (i:number, width:number, height:number) => ({key:String(i),layer:'test',vertex,fragment,width,height,calls:[{fn:'1f',name:'t',v:[i/7]}],stage:'test',message:''});
      let differing = 0, compared = 0;
      for (const [width,height] of [[64,36],[320,180],[1920,1080],[80,45]]) {
        const requests = Array.from({length:8},(_,i)=>request(i,width,height));
        const references = requests.map(r=>runner.renderSync(r));
        const running = requests.map(r=>runner.render(r));
        // This task runs while the asynchronous fence waits yield. It uses
        // the same context and a different target size.
        await new Promise(r=>setTimeout(r,1));
        runner.renderSync(request(99,117,63));
        const actual = await Promise.all(running);
        for (let i=0;i<actual.length;i++) {
          if (actual[i].key !== references[i].key) throw new Error('Readback attached to the wrong request');
          for (let j=0;j<actual[i].rgba16f.length;j++) {
            compared++;
            if (actual[i].rgba16f[j] !== references[i].rgba16f[j]) differing++;
          }
        }
      }
      // The integrated defs runner: sampled inputs, chain feedback, three
      // passes, uniform changes, and a snapshot while pass fences yield.
      const sampled = '#version 300 es\nprecision highp float; uniform sampler2D a; uniform sampler2D b; uniform vec2 size; uniform float t; out vec4 color; void main(){vec2 uv=gl_FragCoord.xy/size; color=mix(texture(a,uv),texture(b,uv),t);}';
      for (const [width,height] of [[64,36],[1920,1080]]) {
        const input = new Uint16Array(width*height*4), backdrop = new Uint16Array(input.length);
        for(let j=0;j<input.length;j++){input[j]=j%11 ? 0x3c00 : 0x3800;backdrop[j]=j%7 ? 0x3400 : 0;}
        const requests = Array.from({length:4},(_,i)=>({...request(i,width,height),fragment:sampled,textures:2,
          units:[{unit:0,texture:0,chain:true},{unit:2,texture:1,chain:false}],
          calls:[{fn:'1i',name:'a',v:[0]},{fn:'1i',name:'b',v:[2]},{fn:'2f',name:'size',v:[width,height]},{fn:'1f',name:'t',v:[i/7]}],
          morePasses:[[{fn:'1f',name:'t',v:[.25]}],[{fn:'1f',name:'t',v:[.75]}]]}));
        const references = requests.map(r=>runner.renderSync(r,[input,backdrop]));
        const running = requests.map(r=>runner.render(r,[new Uint16Array(input),new Uint16Array(backdrop)]));
        await new Promise(r=>setTimeout(r,1));
        runner.renderSync(request(88,117,63));
        const actual = await Promise.all(running);
        for(let i=0;i<actual.length;i++) for(let j=0;j<actual[i].rgba16f.length;j++) {
          compared++; if(actual[i].rgba16f[j]!==references[i].rgba16f[j]) differing++;
        }
        // Reusing a caller buffer after completion must upload changed pixels.
        input.fill(0x3000);
        const changed = runner.renderSync(requests[0],[input,backdrop]);
        const changedAsync = await runner.render(requests[0],[input,backdrop]);
        for(let j=0;j<changed.rgba16f.length;j++){compared++;if(changed.rgba16f[j]!==changedAsync.rgba16f[j]) differing++;}
      }
      runner.dispose();
      return { differing, compared };
    });
    expect(result.compared).toBeGreaterThan(100_000_000);
    expect(result.differing).toBe(0);
    expect(app.diagnostics.pageErrors).toEqual([]);
  } finally { await app.close(); }
});

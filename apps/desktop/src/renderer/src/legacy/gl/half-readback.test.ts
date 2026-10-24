import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AlphaReadbackRetryError, createAlphaReadback, createFloatReadback, createHalfReadback, type DeferredAlphaReader } from './half-readback';

function half(bits: number): number {
  const sign = bits & 32768 ? -1 : 1, exponent = bits >> 10 & 31, mantissa = bits & 1023;
  return exponent === 31 ? mantissa ? NaN : sign * Infinity
    : sign * (exponent ? (1 + mantissa / 1024) * 2 ** (exponent - 15) : mantissa * 2 ** -24);
}

function setup(depth: 16 | 32 = 16) {
  const names = ['ACTIVE_TEXTURE','TEXTURE0','TEXTURE_BINDING_2D','SAMPLER_BINDING','CURRENT_PROGRAM','VERTEX_ARRAY_BINDING','ARRAY_BUFFER_BINDING',
    'READ_FRAMEBUFFER_BINDING','DRAW_FRAMEBUFFER_BINDING','RENDERBUFFER_BINDING','VIEWPORT','COLOR_WRITEMASK','BLEND','DITHER','DEPTH_TEST','SCISSOR_TEST',
    'CULL_FACE','STENCIL_TEST','RASTERIZER_DISCARD','SAMPLE_ALPHA_TO_COVERAGE','SAMPLE_COVERAGE','PIXEL_PACK_BUFFER_BINDING','PIXEL_UNPACK_BUFFER_BINDING',
    'PACK_ALIGNMENT','PACK_ROW_LENGTH','PACK_SKIP_ROWS','PACK_SKIP_PIXELS','UNPACK_ALIGNMENT','UNPACK_ROW_LENGTH','UNPACK_SKIP_ROWS','UNPACK_SKIP_PIXELS',
    'UNPACK_FLIP_Y_WEBGL','UNPACK_PREMULTIPLY_ALPHA_WEBGL','FRAGMENT_SHADER','VERTEX_SHADER','HIGH_FLOAT','COMPILE_STATUS','LINK_STATUS','ARRAY_BUFFER',
    'STATIC_DRAW','BUFFER_SIZE','FLOAT','RENDERBUFFER','RGBA8UI','FRAMEBUFFER','COLOR_ATTACHMENT0','FRAMEBUFFER_COMPLETE','IMPLEMENTATION_COLOR_READ_FORMAT',
    'IMPLEMENTATION_COLOR_READ_TYPE','RGBA_INTEGER','UNSIGNED_BYTE','TEXTURE_2D','TEXTURE_MIN_FILTER','TEXTURE_MAG_FILTER','NEAREST','TEXTURE_WRAP_S',
    'TEXTURE_WRAP_T','CLAMP_TO_EDGE','PIXEL_PACK_BUFFER','PIXEL_UNPACK_BUFFER','RGBA16F','RGBA','HALF_FLOAT','TRIANGLE_STRIP','READ_FRAMEBUFFER',
    'DRAW_FRAMEBUFFER','RENDERBUFFER_WIDTH','RENDERBUFFER_HEIGHT','RENDERBUFFER_INTERNAL_FORMAT','RGBA32F','HIGH_INT','MAX_RENDERBUFFER_SIZE',
    'STREAM_READ','SYNC_GPU_COMMANDS_COMPLETE','ALREADY_SIGNALED','CONDITION_SATISFIED','TIMEOUT_EXPIRED','WAIT_FAILED'];
  const gl: any = Object.fromEntries(names.map((name, i) => [name, i + 1])); gl.NO_ERROR = 0;
  const state = new Map<number, any>(), enabled = new Set<number>();
  const originalTexture = {}, originalSampler = {}, uploaded = new Map<object, Uint16Array | Float32Array>();
  const buffers = new Map<object, number>(), storage = new Map<object, {w:number;h:number;format:number}>();
  const packed = new Map<object, Uint8Array>();
  const probeReads: Float32Array[] = [], draws: object[] = [];
  let bindingTexture: object | null = originalTexture, bindingSampler: object | null = originalSampler;
  let current = true, lost = false, incompatible: boolean | 16 | 32 = false, failCopy = false, failActualSize = false, silentRead = false, silentDraw = false;
  let matchesDepth = true, status = gl.ALREADY_SIGNALED;
  let nextHeight=0, drawnHeight=0, nextNonce:number[]=[], drawnNonce:number[]=[], drawnTexture:object|null=null;
  const source = { actual: true }, actual = new Float32Array([.5, .25, -1, 1, NaN, 2, 1/1024, .125]);
  state.set(gl.ACTIVE_TEXTURE, gl.TEXTURE0 + 3);
  state.set(gl.CURRENT_PROGRAM, {}); state.set(gl.VERTEX_ARRAY_BINDING, {}); state.set(gl.ARRAY_BUFFER_BINDING, {});
  state.set(gl.READ_FRAMEBUFFER_BINDING, {}); state.set(gl.DRAW_FRAMEBUFFER_BINDING, {}); state.set(gl.RENDERBUFFER_BINDING, {});
  state.set(gl.VIEWPORT, [3,4,51,39]); state.set(gl.COLOR_WRITEMASK, [false,true,false,true]);
  state.set(gl.PIXEL_PACK_BUFFER_BINDING, {}); state.set(gl.PIXEL_UNPACK_BUFFER_BINDING, {});
  for (const name of ['BLEND','DITHER','DEPTH_TEST','SCISSOR_TEST','CULL_FACE','STENCIL_TEST','RASTERIZER_DISCARD','SAMPLE_ALPHA_TO_COVERAGE','SAMPLE_COVERAGE']) enabled.add(gl[name]);
  for (const name of ['PACK_ALIGNMENT','UNPACK_ALIGNMENT']) state.set(gl[name], 8);
  for (const name of ['PACK_ROW_LENGTH','PACK_SKIP_ROWS','PACK_SKIP_PIXELS','UNPACK_ROW_LENGTH','UNPACK_SKIP_ROWS','UNPACK_SKIP_PIXELS']) state.set(gl[name], 3);
  state.set(gl.UNPACK_FLIP_Y_WEBGL, true); state.set(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
  Object.assign(gl, {
    getParameter: vi.fn((key:number) => key === gl.TEXTURE_BINDING_2D ? bindingTexture : key === gl.SAMPLER_BINDING ? bindingSampler
      : key === gl.IMPLEMENTATION_COLOR_READ_FORMAT ? gl.RGBA_INTEGER : key === gl.IMPLEMENTATION_COLOR_READ_TYPE ? gl.UNSIGNED_BYTE
        : key === gl.MAX_RENDERBUFFER_SIZE ? 8192 : state.get(key)),
    getShaderPrecisionFormat: vi.fn(() => ({precision:23,rangeMin:127,rangeMax:127})), isContextLost: vi.fn(() => lost),
    activeTexture: vi.fn((unit:number) => state.set(gl.ACTIVE_TEXTURE,unit)), bindTexture: vi.fn((_target:number,texture:object|null) => { bindingTexture=texture; }),
    bindSampler: vi.fn((_unit:number,sampler:object|null) => { bindingSampler=sampler; }),
    useProgram: vi.fn((value:object) => state.set(gl.CURRENT_PROGRAM,value)), bindVertexArray: vi.fn((value:object) => state.set(gl.VERTEX_ARRAY_BINDING,value)),
    bindBuffer: vi.fn((target:number,value:object) => state.set(target===gl.ARRAY_BUFFER?gl.ARRAY_BUFFER_BINDING:target===gl.PIXEL_PACK_BUFFER?gl.PIXEL_PACK_BUFFER_BINDING:gl.PIXEL_UNPACK_BUFFER_BINDING,value)),
    bindFramebuffer: vi.fn((target:number,value:object) => {if(target!==gl.DRAW_FRAMEBUFFER)state.set(gl.READ_FRAMEBUFFER_BINDING,value);if(target!==gl.READ_FRAMEBUFFER)state.set(gl.DRAW_FRAMEBUFFER_BINDING,value);}),
    bindRenderbuffer: vi.fn((_target:number,value:object) => state.set(gl.RENDERBUFFER_BINDING,value)),
    viewport: vi.fn((...value:number[]) => state.set(gl.VIEWPORT,value)), colorMask: vi.fn((...value:boolean[]) => state.set(gl.COLOR_WRITEMASK,value)),
    enable: vi.fn((cap:number) => enabled.add(cap)), disable: vi.fn((cap:number) => enabled.delete(cap)), isEnabled: vi.fn((cap:number) => enabled.has(cap)), pixelStorei: vi.fn((key:number,value:any) => state.set(key,value)),
    createShader: vi.fn(() => ({})), shaderSource: vi.fn(), compileShader: vi.fn(), getShaderParameter: vi.fn(() => true), deleteShader: vi.fn(),
    createProgram: vi.fn(() => ({})), attachShader: vi.fn(), linkProgram: vi.fn(), getProgramParameter: vi.fn(() => true), deleteProgram: vi.fn(),
    createVertexArray: vi.fn(() => ({})), deleteVertexArray: vi.fn(), createBuffer: vi.fn(() => ({})), deleteBuffer: vi.fn(),
    bufferData: vi.fn((target:number,data:Float32Array|number) => {
      const buffer=state.get(target===gl.ARRAY_BUFFER?gl.ARRAY_BUFFER_BINDING:gl.PIXEL_PACK_BUFFER_BINDING);
      buffers.set(buffer,typeof data==='number'?data:data.byteLength);
      if(typeof data==='number')packed.set(buffer,new Uint8Array(data));
    }),
    getBufferParameter: vi.fn((target:number) => buffers.get(state.get(target===gl.ARRAY_BUFFER?gl.ARRAY_BUFFER_BINDING:gl.PIXEL_PACK_BUFFER_BINDING))),
    getAttribLocation: vi.fn(() => 0), enableVertexAttribArray: vi.fn(), vertexAttribPointer: vi.fn(),
    createFramebuffer: vi.fn(() => ({})), deleteFramebuffer: vi.fn(), createRenderbuffer: vi.fn(() => ({})), deleteRenderbuffer: vi.fn(),
    renderbufferStorage: vi.fn((_target:number,format:number,w:number,h:number) => {if(!failActualSize||w===257)storage.set(state.get(gl.RENDERBUFFER_BINDING),{w,h,format});}),
    framebufferRenderbuffer: vi.fn(), checkFramebufferStatus: vi.fn(() => gl.FRAMEBUFFER_COMPLETE),
    getRenderbufferParameter: vi.fn((_target:number,key:number) => {const s=storage.get(state.get(gl.RENDERBUFFER_BINDING));return key===gl.RENDERBUFFER_WIDTH?s?.w:key===gl.RENDERBUFFER_HEIGHT?s?.h:s?.format;}),
    createTexture: vi.fn(() => ({})), deleteTexture: vi.fn(), texParameteri: vi.fn(),
    texImage2D: vi.fn((...args:any[]) => {expect(state.get(gl.PIXEL_UNPACK_BUFFER_BINDING)).toBeNull();uploaded.set(bindingTexture!,args[8]);}),
    framebufferTexture2D: vi.fn(), getUniformLocation: vi.fn((_program:object,name:string) => name), uniform1i: vi.fn(),
    uniform1ui:vi.fn((_location:string,value:number)=>{nextHeight=value;}),uniform4ui:vi.fn((_location:string,...value:number[])=>{nextNonce=value;}),
    drawArrays: vi.fn(() => {for(const cap of enabled)expect(gl.isEnabled(cap)).toBe(false);expect(bindingSampler).toBeNull();draws.push(bindingTexture!);}),
    readPixels: vi.fn((_x:number,_y:number,_w:number,_h:number,format:number,_type:number,destination:Float32Array|Uint8Array|number) => {
      const target=typeof destination==='number'?packed.get(state.get(gl.PIXEL_PACK_BUFFER_BINDING))!:destination;
      if(typeof destination==='number')expect(destination).toBe(0);
      else expect(state.get(gl.PIXEL_PACK_BUFFER_BINDING)).toBeNull();
      const bits=uploaded.get(format===gl.RGBA?bindingTexture!:drawnTexture!);
      const sample=(i:number)=>bits instanceof Float32Array?bits[i]!:bits?half(bits[i]!):actual[i]!;
      if(format===gl.RGBA){probeReads.push(target as Float32Array);for(let i=0;i<target.length;i++)target[i]=sample(i);}
      else {
        if(drawnTexture===source&&failCopy)throw new Error('GPU read failed');
        if(silentRead)return;
        const visible=_w*drawnHeight*4;
        for(let i=0;i<target.length;i++)target[i]=i>=visible?drawnNonce[i%4]!:Math.round(Math.max(0,Math.min(1,sample(i)))*255);
        if(bits&&(incompatible===true||incompatible===(bits instanceof Float32Array?32:16)))target[0]=target[0]!^1;
      }
    }),
    fenceSync: vi.fn((): object | null => ({})), deleteSync: vi.fn(), flush: vi.fn(),
    clientWaitSync: vi.fn(() => status),
    getBufferSubData: vi.fn((_target:number,_offset:number,target:Uint8Array) => target.set(packed.get(state.get(gl.PIXEL_PACK_BUFFER_BINDING))!)),
    getError: vi.fn(() => gl.NO_ERROR),
  });
  // Drawing must disable every interfering cap, not only those currently enabled.
  gl.drawArrays.mockImplementation(() => {for(const name of ['BLEND','DITHER','DEPTH_TEST','SCISSOR_TEST','CULL_FACE','STENCIL_TEST','RASTERIZER_DISCARD','SAMPLE_ALPHA_TO_COVERAGE','SAMPLE_COVERAGE'])expect(enabled.has(gl[name])).toBe(false);expect(bindingSampler).toBeNull();draws.push(bindingTexture!);if(!silentDraw){drawnTexture=bindingTexture;drawnHeight=nextHeight;drawnNonce=[...nextNonce];}});
  const snapshot=()=>({state:new Map(state),enabled:new Set(enabled),texture:bindingTexture,sampler:bindingSampler});
  const fallback=new Uint8Array([99]);
  const render=vi.fn((_time:number,_options:Record<string,unknown>,read:(texture:WebGLTexture)=>Uint8Array<ArrayBuffer>|null)=>read(source as any)||fallback);
  const generation={};
  const createHalf=(token=generation)=>createHalfReadback(gl,2,1,token,render,()=>current);
  const createFloat=(token=generation)=>createFloatReadback(gl,2,1,token,render,()=>current);
  const create=depth===16?createHalf:createFloat;
  const renderDeferred=vi.fn((_time:number,_options:Record<string,unknown>,read:DeferredAlphaReader)=>read.queue(source as any));
  const createAsync=(token=generation,width=2,height=1)=>createAlphaReadback(gl,width,height,token,depth,renderDeferred,()=>current,()=>matchesDepth);
  return {gl,create,createHalf,createFloat,createAsync,render,renderDeferred,generation,snapshot,source,probeReads,draws,uploaded,fallback,actual,
    depthMismatch:()=>{matchesDepth=false;},status:(next:number)=>{status=next;},
    mismatch:(only?:16|32)=>{incompatible=only??true;}, lose:()=>{lost=true;}, replace:()=>{current=false;}, failCopy:()=>{failCopy=true;}, failActualSize:()=>{failActualSize=true;},silentRead:()=>{silentRead=true;},silentDraw:()=>{silentDraw=true;}};
}

describe('half-float integer readback', () => {
  it.each([16,32] as const)('rejects a %i-bit target without room for its guard row without disabling smaller captures', depth => {
    const h=setup(depth),get=h.gl.getParameter.getMockImplementation()!,factory=depth===16?createHalfReadback:createFloatReadback;
    h.gl.getParameter.mockImplementation((key:number)=>key===h.gl.MAX_RENDERBUFFER_SIZE?512:get(key));
    const create=(width:number,height:number)=>factory(h.gl,width,height,h.generation,h.render,()=>true);
    expect(create(513,128)).toBeNull();expect(create(128,512)).toBeNull();expect(create(512,512)).toBeNull();
    expect(h.gl.getShaderPrecisionFormat).not.toHaveBeenCalled();expect(h.gl.createProgram).not.toHaveBeenCalled();
    const boundary=create(512,511)!;expect(boundary).not.toBeNull();boundary.dispose();
    const smaller=h.create()!;expect(smaller.capture(0)).not.toBe(h.fallback);smaller.dispose();
    expect(h.probeReads).toHaveLength(1);
  });

  it('proves all half patterns on the actual generation, restores nondefault GL state, and reuses one raw frame', () => {
    const h=setup(),before=h.snapshot(),session=h.create()!;
    expect(h.gl.createProgram).not.toHaveBeenCalled(); // Lazy: only actual16-bit capture invokes readHalf.
    const first=session.capture(0,{transparent:true}),second=session.capture(.5);
    expect(first).toBe(second);expect([...first!]).toEqual([128,64,0,255,0,255,0,32]);
    expect(h.probeReads).toHaveLength(1);expect(h.probeReads[0]).toHaveLength(257*256*4);
    const patterns=[...h.uploaded.values()][0]!;
    for(let channel=0;channel<4;channel++)expect(new Set(Array.from({length:65536},(_,i)=>patterns[i*4+channel])).size).toBe(65536);
    expect(h.snapshot()).toEqual(before);
    expect(h.gl.shaderSource.mock.calls.some(([,source]:any[])=>source.includes('uniform highp sampler2D'))).toBe(true);
    session.dispose();session.dispose();
    expect(h.gl.deleteTexture).toHaveBeenCalledOnce();expect(h.gl.deleteFramebuffer).toHaveBeenCalledTimes(2);
    expect(h.gl.deleteProgram).toHaveBeenCalledOnce();expect(h.gl.deleteShader).toHaveBeenCalledTimes(2);
    expect(h.gl.deleteBuffer).toHaveBeenCalledOnce();expect(h.gl.deleteVertexArray).toHaveBeenCalledOnce();expect(h.gl.deleteRenderbuffer).toHaveBeenCalledOnce();
  });

  it('caches only the boolean per init token, reproves a new generation, and rejects invalidated sessions', () => {
    const h=setup(),a=h.create()!;a.capture(0);a.dispose();
    const b=h.create()!;b.capture(0);b.dispose();expect(h.probeReads).toHaveLength(1);
    const c=h.create({})!;c.capture(0);expect(h.probeReads).toHaveLength(2);
    h.replace();expect(()=>c.capture(1)).toThrow('GPU');c.dispose();
  });

  it('falls back and caches a one-byte proof mismatch without a second render', () => {
    const h=setup(),before=h.snapshot(),session=h.create()!;h.mismatch();
    expect(session.capture(0)).toBe(h.fallback);expect(session.capture(1)).toBe(h.fallback);
    expect(h.render).toHaveBeenCalledTimes(2);expect(h.probeReads).toHaveLength(1);expect(h.snapshot()).toEqual(before);
    expect(h.create()).toBeNull();session.dispose();expect(h.gl.deleteProgram).toHaveBeenCalledOnce();
  });

  it.each(['precision','null'] as const)('rejects unsupported precision or context before allocating: %s', reason => {
    const h=setup();if(reason==='precision')h.gl.getShaderPrecisionFormat.mockReturnValue({precision:16,rangeMin:127,rangeMax:127});
    expect(reason==='null'?createHalfReadback(null,2,1,{},h.render,()=>true):h.create()).toBeNull();
    expect(h.gl.createProgram).not.toHaveBeenCalled();expect(h.render).not.toHaveBeenCalled();
  });

  it.each(['shader','link','buffer','framebuffer','renderbuffer','incomplete','pair','probe-texture','probe-read'] as const)('falls back and cleans resources when setup/proof fails: %s', reason => {
    const h=setup(),before=h.snapshot(),session=h.create()!;
    if(reason==='shader')h.gl.getShaderParameter.mockReturnValue(false);
    if(reason==='link')h.gl.getProgramParameter.mockReturnValue(false);
    if(reason==='buffer')h.gl.getBufferParameter.mockReturnValue(0);
    if(reason==='framebuffer')h.gl.createFramebuffer.mockReturnValueOnce(null);
    if(reason==='renderbuffer')h.gl.createRenderbuffer.mockReturnValueOnce(null);
    if(reason==='incomplete')h.gl.checkFramebufferStatus.mockReturnValue(0);
    if(reason==='pair'){const get=h.gl.getParameter.getMockImplementation()!;h.gl.getParameter.mockImplementation((key:number)=>key===h.gl.IMPLEMENTATION_COLOR_READ_TYPE?123456:get(key));}
    if(reason==='probe-texture')h.gl.createTexture.mockReturnValueOnce(null);
    if(reason==='probe-read')h.gl.readPixels.mockImplementationOnce(()=>{throw new Error('Probe read failed');});
    expect(session.capture(0)).toBe(h.fallback);expect(h.snapshot()).toEqual(before);session.dispose();
    expect(h.gl.deleteProgram).toHaveBeenCalledTimes(h.gl.createProgram.mock.calls.length);
    expect(h.gl.deleteShader).toHaveBeenCalledTimes(h.gl.createShader.mock.calls.length);
  });

  it('detects storage OOM that leaves the old smaller renderbuffer complete', () => {
    const h=setup(),session=h.create()!,before=h.snapshot();h.failActualSize();
    expect(session.capture(0)).toBe(h.fallback);expect(h.snapshot()).toEqual(before);
    expect(h.gl.readPixels.mock.calls.filter((args:any[])=>args[2]===2)).toHaveLength(0);
    session.dispose();
  });

  it.each(['read','silent-read','silent-draw'] as const)('falls back to the same source frame when actual copy fails and restores state: %s', reason => {
    const h=setup(),session=h.create()!;session.capture(0);const before=h.snapshot();
    if(reason==='read')h.failCopy();
    if(reason==='silent-read')h.silentRead();
    if(reason==='silent-draw')h.silentDraw();
    expect(session.capture(1)).toBe(h.fallback);expect(h.snapshot()).toEqual(before);expect(session.capture(2)).toBe(h.fallback);session.dispose();
  });

  it('preserves preexisting WebGL errors while detecting context loss during a copy', () => {
    const h=setup(),session=h.create()!;h.gl.getError.mockReturnValue(1282);
    expect(session.capture(0)).not.toBe(h.fallback);expect(h.gl.getError).not.toHaveBeenCalled();
    const before=h.snapshot(),read=h.gl.readPixels.getMockImplementation()!;
    h.gl.readPixels.mockImplementation((...args:any[])=>{read(...args);h.lose();});
    expect(()=>session.capture(1)).toThrow('GPU');expect(h.snapshot()).toEqual(before);session.dispose();
  });

  it('rejects reentrant capture and use after disposal without retaining resources', () => {
    const h=setup(),session=h.create()!;
    h.render.mockImplementationOnce((_time,_opts,read)=>{expect(()=>session.capture(1)).toThrow('already in progress');return read(h.source as any)!;});
    session.capture(0);session.dispose();expect(()=>session.capture(1)).toThrow('GPU');
  });
});

describe('32-bit float integer readback', () => {
  it('probes Float32 midpoint neighbours and every exponent class in each channel, restores state, and reuses one output', () => {
    const h=setup(32),before=h.snapshot(),session=h.create()!;
    // The first value rounds up if the final addition happens in Float32.
    h.actual.set(new Float32Array(new Uint32Array([0x3b008080,0x3b008081,0x80000000,0xff800000,0x7f800000,0x7f800001,0x3f000000,0x3f7fffff]).buffer));
    expect(Math.floor(Math.fround(Math.fround(h.actual[0]!*255)+.5))).toBe(1);
    const first=session.capture(0),second=session.capture(.5);
    expect(first).toBe(second);expect([...first!]).toEqual([0,1,0,0,255,0,128,255]);
    expect(h.probeReads).toHaveLength(1);expect(h.probeReads[0]).toHaveLength(257*256*4);
    const uploaded=[...h.uploaded.values()][0]!;
    expect(uploaded).toBeInstanceOf(Float32Array);
    expect(h.gl.texImage2D.mock.calls[0]![2]).toBe(h.gl.RGBA32F);
    expect(h.gl.texImage2D.mock.calls[0]![7]).toBe(h.gl.FLOAT);
    const bits=new Uint32Array(uploaded.buffer),sets=Array.from({length:4},(_,c)=>new Set(Array.from({length:257*256},(_,p)=>bits[p*4+c]!)));
    const scratch=new Float32Array(1),encoded=new Uint32Array(scratch.buffer);
    for(const values of sets){
      for(let byte=0;byte<255;byte++){
        scratch[0]=(byte+.5)/255;
        for(const delta of[-16,-1,0,1,16]){
          expect(values.has(encoded[0]!+delta)).toBe(true);
          expect(values.has((encoded[0]!+delta+0x80000000)>>>0)).toBe(true);
        }
      }
      for(let exponent=0;exponent<256;exponent++)for(const fraction of[0,1,0x3fffff,0x400000,0x7ffffe,0x7fffff]){
        expect(values.has(exponent*0x800000+fraction)).toBe(true);
        expect(values.has((exponent*0x800000+fraction+0x80000000)>>>0)).toBe(true);
      }
    }
    expect(h.snapshot()).toEqual(before);expect(h.gl.getError).not.toHaveBeenCalled();
    session.dispose();session.dispose();
    expect(h.gl.deleteTexture).toHaveBeenCalledOnce();expect(h.gl.deleteFramebuffer).toHaveBeenCalledTimes(2);
    expect(h.gl.deleteProgram).toHaveBeenCalledOnce();expect(h.gl.deleteShader).toHaveBeenCalledTimes(2);
    expect(h.gl.deleteBuffer).toHaveBeenCalledOnce();expect(h.gl.deleteVertexArray).toHaveBeenCalledOnce();expect(h.gl.deleteRenderbuffer).toHaveBeenCalledOnce();
  });

  it.each([16,32] as const)('keeps failed %i-bit proof results independent from the other depth on the same generation', depth => {
    const h=setup(depth);h.mismatch(depth);
    const failed=h.create()!;expect(failed.capture(0)).toBe(h.fallback);failed.dispose();
    expect(h.create()).toBeNull();
    const other=(depth===16?h.createFloat:h.createHalf)()!;
    expect(other.capture(0)).not.toBe(h.fallback);other.dispose();
    expect(h.probeReads).toHaveLength(2);
    const repeat=(depth===16?h.createFloat:h.createHalf)()!;
    expect(repeat.capture(1)).not.toBe(h.fallback);repeat.dispose();expect(h.probeReads).toHaveLength(2);
  });

  it('caches the 32-bit proof by init generation and rejects replaced contexts and disposed sessions', () => {
    const h=setup(32),a=h.create()!;a.capture(0);a.dispose();
    const b=h.create()!;b.capture(0);b.dispose();expect(h.probeReads).toHaveLength(1);
    const c=h.create({})!;c.capture(0);expect(h.probeReads).toHaveLength(2);
    h.replace();expect(()=>c.capture(1)).toThrow('GPU');c.dispose();expect(()=>c.capture(2)).toThrow('GPU');
  });

  it.each(['float-range','int-range'] as const)('rejects insufficient bit representation before allocation: %s', reason => {
    const h=setup(32),original=h.gl.getShaderPrecisionFormat.getMockImplementation()!;
    h.gl.getShaderPrecisionFormat.mockImplementation((stage:number,kind:number)=>reason==='int-range'&&kind===h.gl.HIGH_INT
      ?{precision:0,rangeMin:15,rangeMax:14}:reason==='float-range'&&kind===h.gl.HIGH_FLOAT
        ?{precision:23,rangeMin:24,rangeMax:24}:original(stage,kind));
    expect(h.create()).toBeNull();expect(h.gl.createProgram).not.toHaveBeenCalled();expect(h.render).not.toHaveBeenCalled();
    expect(h.createHalf()).not.toBeNull(); // A 32-bit precision failure must not disable 16-bit readback.
  });

  it.each(['shader','buffer','incomplete','pair','probe-read','mismatch'] as const)('restores state and uses the same captured source after 32-bit proof/setup failure: %s', reason => {
    const h=setup(32),before=h.snapshot(),session=h.create()!;
    if(reason==='shader')h.gl.getShaderParameter.mockReturnValue(false);
    if(reason==='buffer')h.gl.getBufferParameter.mockReturnValue(0);
    if(reason==='incomplete')h.gl.checkFramebufferStatus.mockReturnValue(0);
    if(reason==='pair'){const get=h.gl.getParameter.getMockImplementation()!;h.gl.getParameter.mockImplementation((key:number)=>key===h.gl.IMPLEMENTATION_COLOR_READ_TYPE?123456:get(key));}
    if(reason==='probe-read')h.gl.readPixels.mockImplementationOnce(()=>{throw new Error('Probe read failed');});
    if(reason==='mismatch')h.mismatch();
    expect(session.capture(0)).toBe(h.fallback);expect(session.capture(1)).toBe(h.fallback);
    expect(h.render).toHaveBeenCalledTimes(2);expect(h.snapshot()).toEqual(before);expect(h.create()).toBeNull();session.dispose();
    expect(h.gl.deleteProgram).toHaveBeenCalledTimes(h.gl.createProgram.mock.calls.length);
    expect(h.gl.deleteShader).toHaveBeenCalledTimes(h.gl.createShader.mock.calls.length);
  });

  it.each(['read','silent-read','silent-draw','storage'] as const)('falls back without rerendering or accepting invalid pixels after an actual 32-bit copy failure: %s', reason => {
    const h=setup(32),session=h.create()!,before=h.snapshot();
    if(reason==='storage')h.failActualSize();else session.capture(0);
    if(reason==='read')h.failCopy();
    if(reason==='silent-read')h.silentRead();
    if(reason==='silent-draw')h.silentDraw();
    const captures=h.render.mock.calls.length;
    expect(session.capture(1)).toBe(h.fallback);expect(h.render).toHaveBeenCalledTimes(captures+1);
    expect(h.snapshot()).toEqual(before);expect(session.capture(2)).toBe(h.fallback);session.dispose();
  });

  it('rejects context loss during 32-bit copy and releases resources without consuming preexisting errors', () => {
    const h=setup(32),session=h.create()!;h.gl.getError.mockReturnValue(1282);session.capture(0);
    const before=h.snapshot(),read=h.gl.readPixels.getMockImplementation()!;
    h.gl.readPixels.mockImplementation((...args:any[])=>{read(...args);h.lose();});
    expect(()=>session.capture(1)).toThrow('GPU');expect(h.snapshot()).toEqual(before);expect(h.gl.getError).not.toHaveBeenCalled();
    session.dispose();expect(h.gl.deleteProgram).toHaveBeenCalledOnce();expect(h.gl.deleteRenderbuffer).toHaveBeenCalledOnce();
  });
});

describe('deferred alpha integer readback', () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

  it.each([16, 32] as const)('queues %i-bit pixels without a CPU frame and restores state before every await', async depth => {
    const h = setup(depth), before = h.snapshot(), session = h.createAsync()!;
    expect(session.byteLength).toBe(16); // Two visible pixels and their guard row.
    expect(h.renderDeferred).not.toHaveBeenCalled();
    expect(h.gl.readPixels.mock.calls.filter((args: any[]) => args[2] === 2)).toHaveLength(0);
    expect(h.snapshot()).toEqual(before);
    const options = { transparent: true, mblur: false, mbSamples: 20, shutter: -.5 };
    const ticket = session.begin(.25, options)!;
    expect(h.renderDeferred).toHaveBeenCalledWith(.25, options, expect.objectContaining({ queue: expect.any(Function) }));
    expect(h.gl.readPixels).toHaveBeenLastCalledWith(0, 0, 2, 2, h.gl.RGBA_INTEGER, h.gl.UNSIGNED_BYTE, 0);
    expect(h.snapshot()).toEqual(before);
    expect(h.gl.getBufferSubData).not.toHaveBeenCalled();
    const target = new Uint8Array(session.byteLength), reading = ticket.readInto(target, () => false);
    expect(h.gl.clientWaitSync).not.toHaveBeenCalled();
    expect(h.snapshot()).toEqual(before);
    await vi.runAllTimersAsync();
    expect(await reading).toBe(true);
    expect([...target.subarray(0, 8)]).toEqual([128, 64, 0, 255, 0, 255, 0, 32]);
    expect(h.snapshot()).toEqual(before);
    const second = session.begin(.5)!, again = second.readInto(target, () => false);
    await vi.runAllTimersAsync();
    expect(await again).toBe(true);
    expect(h.gl.getBufferSubData.mock.calls.map((args: any[]) => args[2])).toEqual([target, target]);
    expect(h.probeReads).toHaveLength(1);
    expect(h.gl.getError).not.toHaveBeenCalled();
    session.dispose(); session.dispose();
    expect(h.gl.deleteBuffer).toHaveBeenCalledTimes(2); // Quad plus one PBO.
    expect(h.gl.deleteProgram).toHaveBeenCalledOnce();
    expect(h.gl.deleteShader).toHaveBeenCalledTimes(2);
    expect(h.gl.deleteFramebuffer).toHaveBeenCalledTimes(2); // Probe plus target.
    expect(h.gl.deleteRenderbuffer).toHaveBeenCalledOnce();
    expect(h.gl.deleteVertexArray).toHaveBeenCalledOnce();
    expect(h.gl.deleteSync).toHaveBeenCalledTimes(2);
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each(['depth', 'cap', 'cap-with-row', 'unsupported'] as const)('returns null before capture for %s', reason => {
    const h = setup();
    if (reason === 'depth') h.depthMismatch();
    if (reason === 'unsupported') h.gl.fenceSync = undefined;
    const session = reason === 'cap' ? h.createAsync(h.generation, 4096, 4096)
      : reason === 'cap-with-row' ? h.createAsync(h.generation, 4096, 2048) : h.createAsync();
    expect(session).toBeNull();
    expect(h.gl.createProgram).not.toHaveBeenCalled();
    expect(h.renderDeferred).not.toHaveBeenCalled();
  });

  it.each(['proof', 'storage', 'buffer-null', 'buffer-size', 'buffer-throw'] as const)('cleans failed eager setup without rendering: %s', reason => {
    const h = setup(), before = h.snapshot();
    if (reason === 'proof') h.mismatch();
    if (reason === 'storage') h.failActualSize();
    if (reason === 'buffer-null') h.gl.createBuffer.mockReturnValueOnce({}).mockReturnValueOnce(null);
    if (reason === 'buffer-size') {
      const size = h.gl.getBufferParameter.getMockImplementation()!;
      h.gl.getBufferParameter.mockImplementation((target: number, key: number) => target === h.gl.PIXEL_PACK_BUFFER ? 0 : size(target, key));
    }
    if (reason === 'buffer-throw') {
      const allocate = h.gl.bufferData.getMockImplementation()!;
      h.gl.bufferData.mockImplementation((target: number, ...args: any[]) => {
        if (target === h.gl.PIXEL_PACK_BUFFER) throw new Error('Allocation failed');
        return allocate(target, ...args);
      });
    }
    expect(h.createAsync()).toBeNull();
    expect(h.snapshot()).toEqual(before);
    expect(h.renderDeferred).not.toHaveBeenCalled();
    expect(h.gl.deleteProgram).toHaveBeenCalledOnce();
    expect(h.gl.deleteShader).toHaveBeenCalledTimes(2);
    expect(h.gl.deleteVertexArray).toHaveBeenCalledOnce();
    expect(h.gl.deleteRenderbuffer).toHaveBeenCalledOnce();
  });

  it('shares only proof results with synchronous sessions and reproves a new init generation', () => {
    const h = setup(), serial = h.create()!;
    serial.capture(0); serial.dispose();
    const session = h.createAsync()!;
    expect(h.probeReads).toHaveLength(1);
    session.dispose();
    const restored = h.createAsync({})!;
    expect(h.probeReads).toHaveLength(2);
    h.replace();
    expect(() => restored.begin(0)).toThrow('GPU');
    restored.dispose();
  });

  it('reserves a ticket before render, rejects reentrancy and releases mismatched or unused captures', () => {
    const h = setup(), session = h.createAsync()!;
    h.renderDeferred.mockImplementationOnce((_time, _options, read) => {
      expect(() => session.begin(1)).toThrow('already pending');
      read.queue(h.source as any);
    });
    const first = session.begin(0)!;
    expect(() => session.begin(1)).toThrow('already pending');
    first.dispose();
    h.renderDeferred.mockImplementationOnce(() => {});
    expect(session.begin(1)).toBeNull();
    expect(h.gl.deleteSync).toHaveBeenCalledOnce();
    h.depthMismatch();
    expect(session.begin(2)).toBeNull();
    expect(h.renderDeferred).toHaveBeenCalledTimes(2);
    session.dispose();
  });

  it('discards a submission if requested depth changes during rendering', () => {
    const h = setup(), session = h.createAsync()!;
    h.renderDeferred.mockImplementationOnce((_time, _options, read) => {
      read.queue(h.source as any);
      h.depthMismatch();
    });
    expect(session.begin(0)).toBeNull();
    expect(h.gl.deleteSync).toHaveBeenCalledOnce();
    expect(h.gl.getBufferSubData).not.toHaveBeenCalled();
    session.dispose();
  });

  it('preserves a new pack binding established while its fence is pending', async () => {
    const h = setup(), session = h.createAsync()!, ticket = session.begin(0)!;
    const reading = ticket.readInto(new Uint8Array(session.byteLength), () => false), otherBuffer = {};
    h.gl.bindBuffer(h.gl.PIXEL_PACK_BUFFER, otherBuffer);
    const waitingState = h.snapshot();
    await vi.runAllTimersAsync();
    expect(await reading).toBe(true);
    expect(h.snapshot()).toEqual(waitingState);
    session.dispose();
  });

  it.each(['before', 'after', 'fence'] as const)('cleans a failed submission and can retry: %s', reason => {
    const h = setup(), session = h.createAsync()!, before = h.snapshot();
    if (reason === 'fence') h.gl.fenceSync.mockReturnValueOnce(null);
    else h.renderDeferred.mockImplementationOnce((_time, _options, read) => {
      if (reason === 'after') read.queue(h.source as any);
      throw new Error('Render failed');
    });
    expect(() => session.begin(0)).toThrow(reason === 'fence' ? 'GPU' : 'Render failed');
    expect(h.snapshot()).toEqual(before);
    session.begin(1)!.dispose();
    session.dispose();
  });

  it.each(['cancel', 'ticket', 'session'] as const)('settles pending reads without copying on %s', async reason => {
    const h = setup(), session = h.createAsync()!, ticket = session.begin(0)!;
    h.status(h.gl.TIMEOUT_EXPIRED);
    let cancelled = false;
    const reading = ticket.readInto(new Uint8Array(session.byteLength), () => cancelled);
    if (reason === 'cancel') cancelled = true;
    if (reason === 'ticket') ticket.dispose();
    if (reason === 'session') session.dispose();
    await vi.runAllTimersAsync();
    expect(await reading).toBe(false);
    expect(h.gl.getBufferSubData).not.toHaveBeenCalled();
    expect(h.gl.deleteSync).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0);
    session.dispose();
  });

  it.each(['silent-read', 'silent-draw', 'copy', 'last-guard-byte'] as const)('reports healthy %s failure as retry, never cancellation', async reason => {
    const h = setup(), session = h.createAsync()!;
    const target = new Uint8Array(session.byteLength), first = session.begin(0)!.readInto(target, () => false);
    await vi.runAllTimersAsync(); expect(await first).toBe(true);
    if (reason === 'silent-read') h.silentRead();
    if (reason === 'silent-draw') h.silentDraw();
    if (reason === 'copy') h.gl.getBufferSubData.mockImplementationOnce(() => { throw new Error('Copy failed'); });
    if (reason === 'last-guard-byte') {
      const copy = h.gl.getBufferSubData.getMockImplementation()!;
      h.gl.getBufferSubData.mockImplementationOnce((...args: any[]) => {
        copy(...args); args[2][args[2].length - 1] ^= 1;
      });
    }
    const before = h.snapshot(), reading = session.begin(1)!.readInto(target, () => false);
    const rejected = expect(reading).rejects.toBeInstanceOf(AlphaReadbackRetryError);
    await vi.runAllTimersAsync(); await rejected;
    expect(h.snapshot()).toEqual(before);
    expect(session.begin(2)).toBeNull();
    session.dispose();
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each(['lost', 'replaced', 'wait', 'wait-throw', 'lost-copy', 'timeout', 'cancel-throw'] as const)('rejects %s safely and releases the fence', async reason => {
    const h = setup(), session = h.createAsync()!, ticket = session.begin(0)!, before = h.snapshot();
    const cancel = vi.fn(() => false);
    const reading = ticket.readInto(new Uint8Array(session.byteLength), cancel);
    const rejected = expect(reading).rejects.not.toBeInstanceOf(AlphaReadbackRetryError);
    if (reason === 'lost') h.lose();
    if (reason === 'replaced') h.replace();
    if (reason === 'wait') h.status(h.gl.WAIT_FAILED);
    if (reason === 'wait-throw') h.gl.clientWaitSync.mockImplementationOnce(() => { throw new Error('GPU wait failed'); });
    if (reason === 'lost-copy') h.gl.getBufferSubData.mockImplementationOnce(() => { h.lose(); });
    if (reason === 'timeout') vi.spyOn(performance, 'now').mockReturnValue(performance.now() + 10001);
    if (reason === 'cancel-throw') cancel.mockImplementationOnce(() => { throw new Error('Cancel failed'); });
    await vi.runAllTimersAsync(); await rejected;
    expect(h.snapshot()).toEqual(before);
    expect(h.gl.deleteSync).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0);
    session.dispose();
  });

  it('rejects invalid targets and duplicate reads without leaking pending ownership', async () => {
    const h = setup(), session = h.createAsync()!;
    await expect(session.begin(0)!.readInto(new Uint8Array(8), () => false)).rejects.toThrow('wrong size');
    await expect(session.begin(1)!.readInto(new Uint8ClampedArray(16) as any, () => false)).rejects.toThrow('type');
    const ticket = session.begin(2)!, target = new Uint8Array(16), reading = ticket.readInto(target, () => false);
    await expect(ticket.readInto(target, () => false)).rejects.toThrow('already being read');
    await vi.runAllTimersAsync(); expect(await reading).toBe(true);
    session.begin(3)!.dispose(); session.dispose();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('does not queue a read after disposal during render', () => {
    const h = setup(), session = h.createAsync()!;
    h.renderDeferred.mockImplementationOnce((_time, _options, read) => { session.dispose(); read.queue(h.source as any); });
    expect(() => session.begin(0)).toThrow('GPU');
    expect(h.gl.fenceSync).not.toHaveBeenCalled();
    expect(h.gl.readPixels.mock.calls.filter((args: any[]) => args[2] === 2)).toHaveLength(0);
  });
});

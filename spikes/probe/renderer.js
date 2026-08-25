'use strict';

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const round = (value) => Math.round(value * 10) / 10;

function errorText(error) {
  return String(error && (error.stack || error.message) || error).slice(0, 4000);
}

function performanceMemory() {
  const memory = performance.memory;
  if (!memory) return null;
  return {
    jsHeapSizeLimit: Number(memory.jsHeapSizeLimit),
    totalJSHeapSize: Number(memory.totalJSHeapSize),
    usedJSHeapSize: Number(memory.usedJSHeapSize),
  };
}

async function measured(fn) {
  const start = performance.now();
  try {
    const detail = await fn();
    return { ok: true, ms: round(performance.now() - start), detail };
  } catch (error) {
    return { ok: false, ms: round(performance.now() - start), detail: null, error: errorText(error) };
  }
}

async function configSupport(api, config) {
  if (!api || typeof api.isConfigSupported !== 'function') return { supported: false, error: 'API unavailable' };
  try {
    const result = await api.isConfigSupported(config);
    return { supported: Boolean(result.supported), config: result.config };
  } catch (error) {
    return { supported: false, error: errorText(error) };
  }
}

function waitForEvent(target, successEvent, timeoutMs, failureEvent = 'error') {
  return new Promise((resolve, reject) => {
    let settled = false;
    const done = (error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      target.removeEventListener(successEvent, success);
      if (failureEvent) target.removeEventListener(failureEvent, failure);
      error ? reject(error) : resolve();
    };
    const success = () => done();
    const failure = () => {
      const mediaError = target.error;
      const detail = mediaError ? `code ${mediaError.code}: ${mediaError.message || 'media error'}` : `${failureEvent} event`;
      done(new Error(detail));
    };
    const timer = setTimeout(() => done(new Error(`timed out waiting for ${successEvent}`)), timeoutMs);
    target.addEventListener(successEvent, success, { once: true });
    if (failureEvent) target.addEventListener(failureEvent, failure, { once: true });
  });
}

async function decodeFixture(fixture) {
  if (!fixture.exists || !fixture.url) return { file: fixture.file, skipped: 'fixture missing' };
  const video = document.createElement('video');
  video.muted = true;
  video.playsInline = true;
  video.preload = 'metadata';
  try {
    const metadata = waitForEvent(video, 'loadedmetadata', 15_000);
    video.src = fixture.url;
    video.load();
    await metadata;
    if (fixture.kind === 'video' && !(video.videoWidth > 0 && video.videoHeight > 0)) {
      throw new Error(`metadata loaded without a decodable video track (${video.videoWidth}x${video.videoHeight})`);
    }
    const detail = {
      file: fixture.file,
      container: fixture.container,
      video: fixture.video,
      audio: fixture.audio,
      ffprobeColor: fixture.ffprobeColor || null,
      duration: video.duration,
      videoWidth: video.videoWidth,
      videoHeight: video.videoHeight,
    };
    const expected = fixture.expectedColorAt;
    if (expected && Number.isFinite(Number(expected.t))) {
      const seeked = waitForEvent(video, 'seeked', 15_000);
      video.currentTime = Math.max(0, Math.min(Number(expected.t), Math.max(0, video.duration - 0.001)));
      await seeked;
      const canvas = document.createElement('canvas');
      canvas.width = Math.max(1, video.videoWidth);
      canvas.height = Math.max(1, video.videoHeight);
      const context = canvas.getContext('2d', { willReadFrequently: true });
      context.drawImage(video, 0, 0, canvas.width, canvas.height);
      const rgba = Array.from(context.getImageData(Math.floor(canvas.width / 2), Math.floor(canvas.height / 2), 1, 1).data);
      const expectedRgb = expected.rgb.map(Number);
      detail.sample = {
        t: video.currentTime,
        rgb: rgba.slice(0, 3),
        expected: expectedRgb,
        matches: expectedRgb.every((channel, index) => Math.abs(channel - rgba[index]) <= 24),
      };
      if (rgba[0] === 0 && rgba[1] === 216 && rgba[2] === 0 && typeof window.VideoFrame === 'function') {
        try {
          const decodedFrame = new VideoFrame(video);
          detail.sample.decoderColorSpace = decodedFrame.colorSpace && typeof decodedFrame.colorSpace.toJSON === 'function'
            ? decodedFrame.colorSpace.toJSON()
            : null;
          decodedFrame.close();
        } catch (error) {
          detail.sample.decoderColorSpaceError = errorText(error);
        }
      }
    }
    if (fixture.hasAlpha) {
      const explicitPoint = fixture.expectedAlphaAt || fixture.alphaAt;
      const point = explicitPoint || (fixture.alphaLayout === 'right-half-transparent' ? { x: 0.75, y: 0.5 } : {});
      const x = Number.isFinite(Number(point.x)) ? (Number(point.x) <= 1 ? Math.floor(Number(point.x) * (video.videoWidth - 1)) : Math.floor(Number(point.x))) : 0;
      const y = Number.isFinite(Number(point.y)) ? (Number(point.y) <= 1 ? Math.floor(Number(point.y) * (video.videoHeight - 1)) : Math.floor(Number(point.y))) : 0;
      const transparent = document.createElement('canvas');
      transparent.width = Math.max(1, video.videoWidth);
      transparent.height = Math.max(1, video.videoHeight);
      const transparentContext = transparent.getContext('2d', { willReadFrequently: true });
      transparentContext.clearRect(0, 0, transparent.width, transparent.height);
      transparentContext.drawImage(video, 0, 0);
      const rgba = Array.from(transparentContext.getImageData(x, y, 1, 1).data);
      const magenta = document.createElement('canvas');
      magenta.width = transparent.width;
      magenta.height = transparent.height;
      const magentaContext = magenta.getContext('2d', { willReadFrequently: true });
      magentaContext.fillStyle = '#ff00ff';
      magentaContext.fillRect(0, 0, magenta.width, magenta.height);
      magentaContext.drawImage(video, 0, 0);
      detail.alpha = {
        point: { x, y, source: explicitPoint ? 'manifest point' : (fixture.alphaLayout || 'corner') },
        transparentCanvasRgba: rgba,
        magentaCanvasRgba: Array.from(magentaContext.getImageData(x, y, 1, 1).data),
        survived: rgba[3] < 250,
      };
    }
    return { ok: true, ...detail };
  } catch (error) {
    return {
      ok: false,
      file: fixture.file,
      error: errorText(error),
      mediaError: video.error ? { code: video.error.code, message: video.error.message || '' } : null,
    };
  } finally {
    video.removeAttribute('src');
    video.load();
  }
}

async function probeCodecs() {
  const videoCodecs = ['avc1.42E01E', 'avc1.640028', 'hvc1.1.6.L93.B0', 'hev1.1.6.L93.B0', 'vp09.00.31.08', 'vp8', 'av01.0.04M.08'];
  const audioCodecs = ['mp4a.40.2', 'opus', 'mp3'];
  const mediaSource = {};
  const videoDecoder = {};
  const audioDecoder = {};
  for (const codec of videoCodecs) {
    const mime = codec === 'vp8' || codec.startsWith('vp09') || codec.startsWith('av01')
      ? `video/webm; codecs="${codec}"` : `video/mp4; codecs="${codec}"`;
    try { mediaSource[codec] = { mime, supported: Boolean(window.MediaSource && MediaSource.isTypeSupported(mime)) }; }
    catch (error) { mediaSource[codec] = { mime, supported: false, error: errorText(error) }; }
    videoDecoder[codec] = await configSupport(window.VideoDecoder, { codec, codedWidth: 1920, codedHeight: 1080 });
  }
  for (const codec of audioCodecs) {
    audioDecoder[codec] = await configSupport(window.AudioDecoder, { codec, sampleRate: 48_000, numberOfChannels: 2 });
  }
  const fixtureInfo = await window.probe.fixtures();
  const dynamic = [];
  for (const fixture of fixtureInfo.fixtures) dynamic.push(await decodeFixture(fixture));
  return { static: { mediaSource, videoDecoder, audioDecoder }, dynamic: { manifest: fixtureInfo.manifest, fixtures: dynamic } };
}

function adapterInfo(adapter) {
  if (!adapter || !adapter.info) return null;
  const info = adapter.info;
  const keys = ['vendor', 'architecture', 'device', 'description', 'subgroupMinSize', 'subgroupMaxSize', 'isFallbackAdapter'];
  const out = {};
  for (const key of keys) if (info[key] !== undefined) out[key] = info[key];
  return out;
}

async function probeWebGpu() {
  if (!navigator.gpu) throw new Error('navigator.gpu is unavailable');
  const start = performance.now();
  const adapter = await navigator.gpu.requestAdapter({ powerPreference: 'high-performance' });
  if (!adapter) throw new Error('requestAdapter returned null');
  const device = await adapter.requestDevice();
  const canvas = document.createElement('canvas');
  canvas.width = 256;
  canvas.height = 256;
  const context = canvas.getContext('2d');
  const gradient = context.createLinearGradient(0, 0, 256, 256);
  gradient.addColorStop(0, '#ef4444');
  gradient.addColorStop(1, '#3b82f6');
  context.fillStyle = gradient;
  context.fillRect(0, 0, 256, 256);
  const bitmap = await createImageBitmap(canvas);
  const texture = device.createTexture({
    size: [256, 256, 1],
    format: 'rgba8unorm',
    usage: GPUTextureUsage.COPY_DST | GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.RENDER_ATTACHMENT,
  });
  device.queue.copyExternalImageToTexture({ source: bitmap }, { texture }, [256, 256]);
  await device.queue.onSubmittedWorkDone();
  bitmap.close();
  texture.destroy();
  device.destroy();
  return { adapterInfo: adapterInfo(adapter), copyMs: round(performance.now() - start), texture: { width: 256, height: 256, format: 'rgba8unorm' } };
}

async function actualVideoEncode() {
  if (!window.VideoEncoder || !window.VideoFrame) throw new Error('VideoEncoder or VideoFrame unavailable');
  const config = { codec: 'vp09.00.31.08', width: 1920, height: 1080, framerate: 30, bitrate: 16_000_000, latencyMode: 'quality' };
  const support = await VideoEncoder.isConfigSupported(config);
  if (!support.supported) throw new Error('vp09.00.31.08 encoding is unsupported');
  let chunkCount = 0;
  let totalBytes = 0;
  let encoderError = null;
  const encoder = new VideoEncoder({
    output: (chunk) => { chunkCount += 1; totalBytes += chunk.byteLength; },
    error: (error) => { encoderError = error; },
  });
  encoder.configure(config);
  const canvas = new OffscreenCanvas(1920, 1080);
  const context = canvas.getContext('2d');
  const start = performance.now();
  for (let index = 0; index < 30; index += 1) {
    context.fillStyle = `hsl(${index * 12} 80% 50%)`;
    context.fillRect(0, 0, 1920, 1080);
    context.fillStyle = '#ffffff';
    context.font = '96px sans-serif';
    context.fillText(String(index), 80 + index * 10, 180 + index * 8);
    const frame = new VideoFrame(canvas, { timestamp: Math.round(index * 1_000_000 / 30), duration: Math.round(1_000_000 / 30) });
    encoder.encode(frame, { keyFrame: index % 60 === 0 });
    frame.close();
    if (encoder.encodeQueueSize > 12) await sleep(0);
  }
  await encoder.flush();
  encoder.close();
  if (encoderError) throw encoderError;
  return { codec: config.codec, frames: 30, chunkCount, totalBytes, ms: round(performance.now() - start) };
}

async function actualAudioEncode() {
  if (!window.AudioEncoder || !window.AudioData) throw new Error('AudioEncoder or AudioData unavailable');
  const config = { codec: 'opus', sampleRate: 48_000, numberOfChannels: 2, bitrate: 160_000 };
  const support = await AudioEncoder.isConfigSupported(config);
  if (!support.supported) throw new Error('Opus encoding is unsupported');
  let chunkCount = 0;
  let totalBytes = 0;
  let decoderDescriptionProvided = false;
  let encoderError = null;
  const encoder = new AudioEncoder({
    output: (chunk, metadata) => {
      chunkCount += 1;
      totalBytes += chunk.byteLength;
      if (metadata && metadata.decoderConfig && metadata.decoderConfig.description) decoderDescriptionProvided = true;
    },
    error: (error) => { encoderError = error; },
  });
  encoder.configure(config);
  const framesPerChunk = 960;
  const start = performance.now();
  for (let chunkIndex = 0; chunkIndex < 50; chunkIndex += 1) {
    const planar = new Float32Array(framesPerChunk * 2);
    for (let frame = 0; frame < framesPerChunk; frame += 1) {
      const sample = Math.sin(2 * Math.PI * 440 * (chunkIndex * framesPerChunk + frame) / 48_000) * 0.2;
      planar[frame] = sample;
      planar[framesPerChunk + frame] = sample;
    }
    const data = new AudioData({
      format: 'f32-planar',
      sampleRate: 48_000,
      numberOfFrames: framesPerChunk,
      numberOfChannels: 2,
      timestamp: chunkIndex * 20_000,
      data: planar,
    });
    encoder.encode(data);
    data.close();
  }
  await encoder.flush();
  encoder.close();
  if (encoderError) throw encoderError;
  return { codec: config.codec, seconds: 1, chunkCount, totalBytes, decoderDescriptionProvided, ms: round(performance.now() - start) };
}

async function probeEncode() {
  const videoConfigs = ['vp09.00.31.08', 'vp8', 'avc1.42E01E'];
  const support = { video: {}, audio: {} };
  for (const codec of videoConfigs) support.video[codec] = await configSupport(window.VideoEncoder, { codec, width: 1920, height: 1080, framerate: 30, bitrate: 16_000_000 });
  support.audio.opus = await configSupport(window.AudioEncoder, { codec: 'opus', sampleRate: 48_000, numberOfChannels: 2, bitrate: 160_000 });
  const actual = {};
  try { actual.video = { ok: true, ...(await actualVideoEncode()) }; }
  catch (error) { actual.video = { ok: false, error: errorText(error) }; }
  try { actual.audio = { ok: true, ...(await actualAudioEncode()) }; }
  catch (error) { actual.audio = { ok: false, error: errorText(error) }; }
  return { support, actual };
}

async function sandboxVariant() {
  const token = `sandbox-${crypto.randomUUID()}`;
  let parentEval = null;
  let parentEvalError = null;
  try { parentEval = window.eval('1+1'); } catch (error) { parentEvalError = { name: error.name, message: error.message }; }
  const frameResult = await new Promise((resolve, reject) => {
    const frame = document.createElement('iframe');
    frame.hidden = true;
    frame.setAttribute('sandbox', 'allow-scripts');
    const progress = [];
    const timer = setTimeout(() => {
      let parentDocumentThrows = false;
      let parentDocumentError = null;
      try { void frame.contentWindow.document; } catch (error) { parentDocumentThrows = true; parentDocumentError = errorText(error); }
      cleanup(null, {
        kind: 'timeout',
        worker: null,
        fetchBlocked: null,
        progress,
        iframeMessageReceived: false,
        inlineScriptBlockedByParentCsp: progress.length === 0,
        error: progress.length === 0
          ? "srcdoc inline script was blocked by the parent page's script-src (unsafe-inline is absent)"
          : 'srcdoc script started but did not produce a final result',
        parentProbeType: null,
        parentProbeAccessThrows: null,
        parentDocumentAccessThrows: null,
        parentDocumentThrows,
        parentDocumentError,
        opaqueCrossOrigin: parentDocumentThrows,
      });
    }, 2_500);
    const receive = (event) => {
      if (event.source !== frame.contentWindow || !event.data || event.data.token !== token) return;
      if (event.data.kind === 'progress') {
        progress.push(event.data.stage);
        return;
      }
      let parentDocumentThrows = false;
      let parentDocumentError = null;
      try { void frame.contentWindow.document; } catch (error) { parentDocumentThrows = true; parentDocumentError = errorText(error); }
      cleanup(null, { ...event.data, progress, parentDocumentThrows, parentDocumentError, opaqueCrossOrigin: parentDocumentThrows });
    };
    const cleanup = (error, value) => {
      clearTimeout(timer);
      window.removeEventListener('message', receive);
      frame.remove();
      error ? reject(error) : resolve(value);
    };
    window.addEventListener('message', receive);
    const workerCode = "const AsyncFunction=(async()=>{}).constructor;const f=new AsyncFunction('return 6*7');f().then(v=>postMessage({v})).catch(e=>postMessage({error:String(e)}));";
    const escapedWorker = JSON.stringify(workerCode).replace(/</g, '\\u003c');
    frame.srcdoc = `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'; worker-src blob:; connect-src 'none'"><script>
      const token=${JSON.stringify(token)};
      const workerCode=${escapedWorker};
      parent.postMessage({token,kind:'progress',stage:'script-started'},'*');
      let parentProbeType='unreadable',parentProbeAccessThrows=false,parentDocumentAccessThrows=false;
      try{parentProbeType=typeof window.parent.probe}catch(e){parentProbeAccessThrows=true}
      try{void parent.document}catch(e){parentDocumentAccessThrows=true}
      const url=URL.createObjectURL(new Blob([workerCode],{type:'text/javascript'}));
      const workerResult=new Promise(resolve=>{const w=new Worker(url);URL.revokeObjectURL(url);const timer=setTimeout(()=>{w.terminate();resolve({error:'worker timed out'})},6000);w.onmessage=e=>{clearTimeout(timer);w.terminate();resolve(e.data)};w.onerror=e=>{clearTimeout(timer);w.terminate();resolve({error:String(e.message||e)})}});
      const fetchResult=fetch('https://example.com').then(()=>({fetchBlocked:false}),e=>({fetchBlocked:true,fetchError:String(e&&e.message||e)}));
      Promise.all([workerResult,fetchResult]).then(([worker,fetchInfo])=>parent.postMessage({token,kind:'final',worker,...fetchInfo,parentProbeType,parentProbeAccessThrows,parentDocumentAccessThrows},'*'));
    <\/script>`;
    document.body.appendChild(frame);
  });
  return {
    answer42: Boolean(frameResult.worker && frameResult.worker.v === 42),
    worker: frameResult.worker,
    fetchBlocked: frameResult.fetchBlocked,
    fetchError: frameResult.fetchError,
    parentProbeType: frameResult.parentProbeType,
    parentProbeAccessThrows: frameResult.parentProbeAccessThrows,
    childParentDocumentThrows: frameResult.parentDocumentAccessThrows,
    parentDocumentThrows: frameResult.parentDocumentThrows,
    opaqueCrossOrigin: frameResult.opaqueCrossOrigin,
    parentEval,
    parentEvalWorks: parentEval === 2,
    parentEvalError,
    iframeMessageReceived: frameResult.iframeMessageReceived !== false,
    inlineScriptBlockedByParentCsp: Boolean(frameResult.inlineScriptBlockedByParentCsp),
    error: frameResult.error || null,
  };
}

const V3_BOOTSTRAP = "(()=>{const variant='V3';addEventListener('message',event=>{if(event.source!==parent)return;const message=event.data||{};if(!message.token||typeof message.code!=='string')return;let parentReachable=true;try{void parent.document}catch(_){parentReachable=false}const source=\"onmessage=e=>{const code=e.data.code;const AF=(async()=>{}).constructor;new AF(code)().then(v=>postMessage({v})).catch(error=>postMessage({error:String(error&&error.message||error)}))}\";const url=URL.createObjectURL(new Blob([source],{type:'text/javascript'}));const workerResult=new Promise(resolve=>{const worker=new Worker(url);URL.revokeObjectURL(url);const timer=setTimeout(()=>{worker.terminate();resolve({error:'worker timed out'})},4000);worker.onmessage=workerEvent=>{clearTimeout(timer);worker.terminate();resolve(workerEvent.data||{})};worker.onerror=workerEvent=>{clearTimeout(timer);worker.terminate();resolve({error:String(workerEvent.message||workerEvent)})};worker.postMessage({code:message.code})});const fetchResult=fetch('https://example.com').then(()=>({fetchBlocked:false}),error=>({fetchBlocked:true,fetchError:String(error&&error.message||error)}));Promise.all([workerResult,fetchResult]).then(([worker,fetchInfo])=>parent.postMessage({token:message.token,kind:'final',worker,parentReachable,...fetchInfo},'*'))},false);parent.postMessage({kind:'sandbox-ready',variant},'*')})();";

function inlineSandboxDocument(variant) {
  const bootstrap = V3_BOOTSTRAP.replace("'V3'", `'${variant}'`);
  return `<!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'; worker-src blob:; connect-src 'none'"><script>${bootstrap}<\/script>`;
}

async function runD2Frame(variant, configure) {
  const token = `d2-${variant}-${crypto.randomUUID()}`;
  const frame = document.createElement('iframe');
  frame.hidden = true;
  frame.setAttribute('sandbox', 'allow-scripts');
  let objectUrl = null;
  let posted = false;
  let received = null;
  let timeoutError = null;
  const postCode = () => {
    if (posted || !frame.contentWindow) return;
    posted = true;
    frame.contentWindow.postMessage({ token, code: 'return 6*7' }, '*');
  };
  const receive = (event) => {
    if (event.source !== frame.contentWindow || !event.data) return;
    if (event.data.kind === 'sandbox-ready') {
      postCode();
      return;
    }
    if (event.data.token === token && event.data.kind === 'final') received = event.data;
  };
  window.addEventListener('message', receive);
  configure(frame, (url) => { objectUrl = url; });
  document.body.appendChild(frame);
  frame.addEventListener('load', () => setTimeout(postCode, 0), { once: true });
  const deadline = performance.now() + 3500;
  while (!received && performance.now() < deadline) await sleep(25);
  if (!received) timeoutError = `${variant} sandbox did not execute its bootstrap or return a result`;
  let opaqueOrigin = false;
  try { void frame.contentWindow.document; } catch (_) { opaqueOrigin = true; }
  const worker = received && received.worker;
  const result = {
    answer42: Boolean(worker && worker.v === 42),
    opaqueOrigin,
    fetchBlocked: Boolean(received && received.fetchBlocked),
    parentReachable: Boolean(received ? received.parentReachable : !opaqueOrigin),
    error: timeoutError || (worker && worker.error) || (received && received.error) || null,
  };
  if (received && received.fetchError) result.fetchError = received.fetchError;
  window.removeEventListener('message', receive);
  frame.remove();
  if (objectUrl) URL.revokeObjectURL(objectUrl);
  return result;
}

async function probeSandboxFollowups() {
  const servedUrl = new URL('sandbox.html', location.href);
  const V1 = await runD2Frame('V1', (frame) => {
    servedUrl.search = '?variant=V1';
    frame.src = servedUrl.href;
  });
  const V1b = await runD2Frame('V1b', (frame) => {
    servedUrl.search = '?variant=V1b';
    frame.src = servedUrl.href;
  });
  const V2 = await runD2Frame('V2', (frame, rememberObjectUrl) => {
    const url = URL.createObjectURL(new Blob([inlineSandboxDocument('V2')], { type: 'text/html' }));
    rememberObjectUrl(url);
    frame.src = url;
  });
  const V3 = await runD2Frame('V3', (frame) => { frame.srcdoc = inlineSandboxDocument('V3'); });
  await window.probe.clearConsoleMessages();
  const V4 = await runD2Frame('V4', (frame) => { frame.srcdoc = inlineSandboxDocument('V4'); });
  await sleep(100);
  const messages = await window.probe.consoleMessages();
  const violations = [...new Set(messages
    .filter((entry) => /Content Security Policy|violates the following/i.test(entry.message))
    .map((entry) => entry.message))];
  V4.cspViolation = violations.length ? violations.join('\n') : null;
  if (V4.cspViolation) V4.error = `${V4.error}; ${V4.cspViolation}`;
  return { V1, V1b, V2, V3, V4 };
}

async function activationAction(label, action, timeoutMs = 30_000) {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'activation';
  button.textContent = label;
  document.body.appendChild(button);
  return new Promise(async (resolve) => {
    let settled = false;
    const finish = (value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      button.remove();
      resolve(value);
    };
    const timer = setTimeout(() => finish({ ok: false, error: 'activation action timed out' }), timeoutMs);
    button.addEventListener('click', async () => {
      try { finish({ ok: true, value: await action() }); }
      catch (error) { finish({ ok: false, error: errorText(error), name: error && error.name, message: error && error.message }); }
    }, { once: true });
    await new Promise(requestAnimationFrame);
    try { await window.probe.syntheticClick(); }
    catch (error) { finish({ ok: false, error: errorText(error) }); }
  });
}

async function queryFontsAttempt() {
  const start = performance.now();
  if (typeof window.queryLocalFonts !== 'function') return { ok: false, ms: 0, name: 'Unavailable', message: 'queryLocalFonts is unavailable' };
  try {
    const fonts = await window.queryLocalFonts();
    return { ok: true, ms: round(performance.now() - start), fontCount: fonts.length, familyCount: new Set(fonts.map((font) => font.family)).size };
  } catch (error) {
    return { ok: false, ms: round(performance.now() - start), name: error.name, message: error.message };
  }
}

async function probeFonts(mainFonts) {
  const withoutActivation = await queryFontsAttempt();
  const withActivationEnvelope = await activationAction('Testing local font permission…', queryFontsAttempt);
  return {
    main: mainFonts,
    renderer: {
      withoutActivation,
      withSyntheticActivation: withActivationEnvelope.ok ? withActivationEnvelope.value : withActivationEnvelope,
    },
    permissionsAsked: await window.probe.permissionLog(),
  };
}

async function probeFsa() {
  const g1 = {
    showSaveFilePicker: typeof window.showSaveFilePicker,
    showOpenFilePicker: typeof window.showOpenFilePicker,
    showDirectoryPicker: typeof window.showDirectoryPicker,
    storageGetDirectory: Boolean(navigator.storage && typeof navigator.storage.getDirectory === 'function'),
  };
  if (window.probe.mode !== 'interactive') return { g1, g2: { skipped: 'interactive mode only' } };
  if (typeof window.showSaveFilePicker !== 'function') return { g1, g2: { ok: false, error: 'showSaveFilePicker is unavailable' } };
  const beforeMain = await window.probe.mainMemory();
  const beforeRenderer = performanceMemory();
  const outcome = await activationAction('Choose where to save the 1 GiB streaming probe file', async () => {
    const handle = await window.showSaveFilePicker({ suggestedName: 'probe-stream.bin' });
    const writable = await handle.createWritable();
    const chunkSize = 8 * 1024 * 1024;
    const chunks = (1024 * 1024 * 1024) / chunkSize;
    const chunk = new Uint8Array(chunkSize);
    chunk.fill(0x5a);
    const start = performance.now();
    for (let index = 0; index < chunks; index += 1) await writable.write(chunk);
    await writable.close();
    const ms = performance.now() - start;
    let deleted = false;
    let deleteError = null;
    if (typeof handle.remove === 'function') {
      try { await handle.remove(); deleted = true; } catch (error) { deleteError = errorText(error); }
    }
    return {
      fileName: handle.name,
      bytes: 1024 * 1024 * 1024,
      ms: round(ms),
      throughputMiBPerSecond: round(1024 / (ms / 1000)),
      deleted,
      deleteError,
      cleanupInstruction: deleted ? null : `Delete ${handle.name} manually from the location selected in the Save dialog.`,
    };
  }, 9 * 60_000);
  const afterMain = await window.probe.mainMemory();
  const afterRenderer = performanceMemory();
  return { g1, g2: { ...outcome, memory: { main: { before: beforeMain, after: afterMain }, renderer: { before: beforeRenderer, after: afterRenderer } } } };
}

function compileShader(gl, type, source) {
  const shader = gl.createShader(type);
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader));
  return shader;
}

async function probeReparent() {
  const stage = document.getElementById('stage');
  const slots = [document.createElement('div'), document.createElement('div')];
  slots[0].style.cssText = 'width:420px;min-height:260px;display:inline-block;vertical-align:top;background:#1f2937';
  slots[1].style.cssText = 'width:360px;min-height:260px;display:inline-block;vertical-align:top;background:#374151';
  stage.replaceChildren(...slots);
  const panel = document.createElement('div');
  panel.style.cssText = 'width:100%;padding:12px;box-sizing:border-box;transition:transform 40ms linear;background:#0f172a';
  const input = document.createElement('input');
  input.type = 'text';
  input.value = 'retained-value';
  const button = document.createElement('button');
  button.textContent = 'counter';
  let clicks = 0;
  button.addEventListener('click', () => { clicks += 1; });
  const canvas = document.createElement('canvas');
  canvas.width = 96;
  canvas.height = 96;
  const gl = canvas.getContext('webgl', { preserveDrawingBuffer: true });
  if (!gl) throw new Error('WebGL unavailable');
  const vertex = compileShader(gl, gl.VERTEX_SHADER, 'attribute vec2 p;void main(){gl_Position=vec4(p,0.,1.);}');
  const fragment = compileShader(gl, gl.FRAGMENT_SHADER, 'precision mediump float;void main(){gl_FragColor=vec4(0.102,0.600,0.902,1.);}');
  const program = gl.createProgram();
  gl.attachShader(program, vertex);
  gl.attachShader(program, fragment);
  gl.linkProgram(program);
  gl.useProgram(program);
  const buffer = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1, 1,-1, -1,1, -1,1, 1,-1, 1,1]), gl.STATIC_DRAW);
  const position = gl.getAttribLocation(program, 'p');
  gl.enableVertexAttribArray(position);
  gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
  const texture = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 2, 2, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([255,0,0,255, 0,255,0,255, 0,0,255,255, 255,255,255,255]));
  gl.viewport(0, 0, canvas.width, canvas.height);
  gl.drawArrays(gl.TRIANGLES, 0, 6);

  const videoSource = document.createElement('canvas');
  videoSource.width = 64;
  videoSource.height = 64;
  const videoContext = videoSource.getContext('2d');
  const video = document.createElement('video');
  video.muted = true;
  video.playsInline = true;
  let videoTimer = null;
  let videoSetupError = null;
  if (typeof videoSource.captureStream === 'function') {
    let tick = 0;
    videoTimer = setInterval(() => { videoContext.fillStyle = tick++ % 2 ? '#22c55e' : '#eab308'; videoContext.fillRect(0, 0, 64, 64); }, 25);
    video.srcObject = videoSource.captureStream(30);
    try { await video.play(); } catch (error) { videoSetupError = errorText(error); }
  } else {
    videoSetupError = 'canvas.captureStream unavailable';
  }
  panel.append(input, button, canvas, video);
  slots[0].appendChild(panel);
  let resizeCount = 0;
  let mutationCount = 0;
  const resizeObserver = new ResizeObserver(() => { resizeCount += 1; });
  const mutationObserver = new MutationObserver((records) => { mutationCount += records.length; });
  resizeObserver.observe(panel);
  mutationObserver.observe(panel, { attributes: true, childList: true, subtree: true });
  const mainWindowFocused = await window.probe.focusWindow();
  await sleep(50);
  input.focus();
  input.setSelectionRange(2, 4);
  const documentFocusedBeforeMoves = document.hasFocus();
  let blurEvents = 0;
  let focusEvents = 0;
  const focusMoves = [];
  input.addEventListener('blur', () => { blurEvents += 1; });
  input.addEventListener('focus', () => { focusEvents += 1; });
  const videoBefore = video.currentTime;
  panel.style.transform = 'translateX(2px)';
  await sleep(60);
  for (let index = 0; index < 50; index += 1) {
    const previouslyFocused = document.activeElement;
    const blurBefore = blurEvents;
    const focusBefore = focusEvents;
    slots[(index + 1) % 2].appendChild(panel);
    const focusLost = previouslyFocused === input && document.activeElement !== previouslyFocused;
    if (focusLost) previouslyFocused.focus({ preventScroll: true });
    focusMoves.push({
      move: index + 1,
      focusLost,
      restored: previouslyFocused !== input || document.activeElement === previouslyFocused,
      blurEvents: blurEvents - blurBefore,
      focusEvents: focusEvents - focusBefore,
      selectionStart: input.selectionStart,
      selectionEnd: input.selectionEnd,
    });
    if (index % 10 === 0) await sleep(0);
  }
  panel.dataset.moved = '50';
  panel.style.width = '99%';
  await sleep(250);
  button.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  const pixel = new Uint8Array(4);
  gl.readPixels(48, 48, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
  const expected = [26, 153, 230, 255];
  const pixelMatches = expected.every((value, index) => Math.abs(value - pixel[index]) <= 3);
  const detail = {
    moves: 50,
    inputValueRetained: input.value === 'retained-value',
    inputValue: input.value,
    focusRetained: document.activeElement === input,
    buttonHandlerRetained: clicks === 1,
    buttonClicks: clicks,
    webglContextLost: gl.isContextLost(),
    webglPixel: Array.from(pixel),
    webglExpected: expected,
    webglPixelMatches: pixelMatches,
    uploadedTextureStillValid: gl.isTexture(texture),
    resizeObserverCount: resizeCount,
    resizeObserverFired: resizeCount > 0,
    mutationObserverCount: mutationCount,
    mutationObserverFired: mutationCount > 0,
    transitionProperty: getComputedStyle(panel).transitionProperty,
    transitionPresent: getComputedStyle(panel).transitionDuration !== '0s',
    videoSetupError,
    videoCurrentTimeBefore: videoBefore,
    videoCurrentTimeAfter: video.currentTime,
    videoKeptPlaying: !videoSetupError && video.currentTime > videoBefore && !video.paused,
    i2: {
      mainWindowFocused,
      documentFocusedBeforeMoves,
      focusRestored: document.activeElement === input && focusMoves.every((move) => move.restored),
      losses: focusMoves.filter((move) => move.focusLost).length,
      restorationAttempts: focusMoves.filter((move) => move.focusLost).length,
      selectionBefore: { start: 2, end: 4 },
      selectionAfter: { start: input.selectionStart, end: input.selectionEnd },
      selectionSurvived: input.selectionStart === 2 && input.selectionEnd === 4,
      blurEvents,
      focusEvents,
      movesWithBlur: focusMoves.filter((move) => move.blurEvents > 0).length,
      movesWithFocus: focusMoves.filter((move) => move.focusEvents > 0).length,
      perMove: focusMoves,
    },
  };
  clearInterval(videoTimer);
  video.pause();
  video.srcObject = null;
  resizeObserver.disconnect();
  mutationObserver.disconnect();
  stage.replaceChildren();
  return detail;
}

async function indexedDbCheck() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open('probe', 1);
    request.onerror = () => reject(request.error || new Error('indexedDB open failed'));
    request.onupgradeneeded = () => { request.result.createObjectStore('values'); };
    request.onsuccess = () => { request.result.close(); resolve(true); };
  });
}

async function probeLargeIpc() {
  const forward = [];
  for (const sizeMiB of [64, 256, 1024]) {
    const size = sizeMiB * 1024 * 1024;
    const beforeRenderer = performanceMemory();
    const start = performance.now();
    try {
      let payload = new Uint8Array(size);
      payload[0] = sizeMiB;
      payload[payload.length - 1] = 255;
      const main = await window.probe.big(payload, Date.now());
      payload = null;
      forward.push({ ok: true, sizeMiB, byteLength: size, roundTripMs: round(performance.now() - start), main, rendererMemory: { before: beforeRenderer, after: performanceMemory() } });
    } catch (error) {
      forward.push({ ok: false, sizeMiB, byteLength: size, ms: round(performance.now() - start), error: errorText(error), rendererMemory: { before: beforeRenderer, after: performanceMemory() } });
    }
    await sleep(100);
  }
  const reverseStart = performance.now();
  let reverse;
  try {
    const rendererBefore = performanceMemory();
    const response = await window.probe.bigReverse(256 * 1024 * 1024);
    reverse = {
      ok: true,
      sizeMiB: 256,
      byteLength: response.payload.byteLength,
      endpoints: [response.payload[0], response.payload[response.payload.byteLength - 1]],
      roundTripMs: round(performance.now() - reverseStart),
      main: response.main,
      rendererMemory: { before: rendererBefore, after: performanceMemory() },
    };
  } catch (error) {
    reverse = { ok: false, sizeMiB: 256, ms: round(performance.now() - reverseStart), error: errorText(error) };
  }
  return { forward, reverse };
}

async function strictEntry() {
  const result = await measured(sandboxVariant);
  await window.probe.strictComplete(result);
}

async function mainEntry() {
  const status = document.getElementById('status');
  const results = {};
  const bootStart = performance.now();
  const snapshot = await window.probe.storeSnapshot();
  const bootBarrier = { ms: round(performance.now() - bootStart), jsonBytes: new TextEncoder().encode(JSON.stringify(snapshot)).byteLength };
  const staticMain = await window.probe.mainStatic();
  const run = async (key, label, fn) => {
    status.textContent = `Running ${key}: ${label}`;
    results[key] = await measured(fn);
    if (key === 'd2' && results[key].detail) {
      for (const variant of ['V1', 'V1b', 'V2', 'V3', 'V4']) results[key][variant] = results[key].detail[variant];
    }
    try { await window.probe.checkpoint(key, results[key]); } catch (_) { /* main may be finalizing */ }
  };

  await run('a', 'codec support and fixture decoding', probeCodecs);
  await run('b', 'WebGPU external image copy', probeWebGpu);
  await run('c', 'WebCodecs encoding', probeEncode);
  await run('d', 'sandbox and CSP', async () => ({ normal: await measured(sandboxVariant), strict: await window.probe.runStrict() }));
  await run('d2', 'sandbox CSP construction follow-ups', probeSandboxFollowups);
  await run('e', 'local and system fonts', () => probeFonts(staticMain.fonts));
  await run('f', 'capturePage', () => window.probe.capturePage());
  await run('g', 'File System Access', probeFsa);
  await run('h', 'Codex discovery', async () => staticMain.codex);
  await run('i', 'panel re-parenting', probeReparent);
  await run('i2', 'focus restoration after panel re-parenting', async () => results.i.detail.i2);
  await run('j', 'app scheme and boot barrier', async () => {
    if (window.probe.mode !== 'app') return { skipped: 'app mode only' };
    await indexedDbCheck();
    return {
      locationOrigin: location.origin,
      isSecureContext,
      cryptoSubtle: typeof crypto.subtle,
      indexedDbOpen: true,
      bootBarrier,
    };
  });
  await run('k', 'large IPC payloads', probeLargeIpc);
  status.textContent = 'Complete. Writing report…';
  window.probe.complete(results);
}

if (location.pathname.endsWith('index-strict.html')) {
  strictEntry().catch((error) => window.probe.strictComplete({ ok: false, detail: null, error: errorText(error) }));
} else {
  mainEntry().catch((error) => {
    document.getElementById('status').textContent = `Fatal renderer error: ${errorText(error)}`;
    window.probe.complete({});
  });
}

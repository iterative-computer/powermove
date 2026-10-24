/* The current exporter's Opus encoding through Chromium's WebCodecs
   AudioEncoder (moved verbatim out of audio.ts): 160 kbps, 20 ms AudioData
   chunks of planar float samples, and OpusHead with a 312-sample pre-skip
   when the encoder gives no description. The job engine's WebCodecs encoder
   (export-encoder/) runs the same function on the engine's mix. */

export function opusHead(channels: any, sampleRate: any) {
  const out: any = new Uint8Array(19);
  out.set([79, 112, 117, 115, 72, 101, 97, 100], 0); // OpusHead
  out[8] = 1;
  out[9] = channels;
  out[10] = 56; out[11] = 1; // 312-sample pre-skip, little endian
  new DataView(out.buffer).setUint32(12, sampleRate, true);
  out[18] = 0;
  return out;
}

export async function supportsOpus() {
  const Encoder: any = window.AudioEncoder;
  const Data: any = window.AudioData;
  if (!Encoder || !Data || typeof Encoder.isConfigSupported !== 'function') return false;
  const config: any = { codec: 'opus', sampleRate: 48000, numberOfChannels: 2, bitrate: 160000 };
  const support: any = await Encoder.isConfigSupported(config).catch(() => null);
  return !!(support && support.supported);
}

export async function encodeOpus(buffer: any) {
  const Encoder: any = window.AudioEncoder;
  const Data: any = window.AudioData;
  if (!buffer || !Encoder || !Data) return null;
  const channels: any = Math.min(2, Math.max(1, buffer.numberOfChannels || 1));
  const config: any = { codec: 'opus', sampleRate: buffer.sampleRate, numberOfChannels: channels, bitrate: 160000 };
  const support: any = await Encoder.isConfigSupported(config).catch(() => null);
  if (!support || !support.supported) return null;
  const chunks: any = [];
  let privateData: any = null;
  let encodeError: any = null;
  const capacity: { wake?: () => void; stop?: () => void } = {};
  const encoder: any = new Encoder({
    output(chunk: any, metadata: any) {
      const description: any = metadata && metadata.decoderConfig && metadata.decoderConfig.description;
      if (!privateData && description) privateData = new Uint8Array(description);
      const bytes: any = new Uint8Array(chunk.byteLength);
      chunk.copyTo(bytes);
      chunks.push({ ts: Number(chunk.timestamp), data: bytes });
    },
    error(error: any) { encodeError = error; capacity.wake?.(); },
  });
  try {
    encoder.configure(config);
    const source: any = Array.from({ length: channels }, (_: any, channel: any) => buffer.getChannelData(channel));
    const frameLength: any = Math.max(1, Math.floor(buffer.sampleRate * .02));
    for (let offset: any = 0; offset < buffer.length; offset += frameLength) {
      if (encodeError) break;
      const frames: any = Math.min(frameLength, buffer.length - offset);
      const planar: any = new Float32Array(frames * channels);
      for (let channel: any = 0; channel < channels; channel++) planar.set(source[channel].subarray(offset, offset + frames), channel * frames);
      const data: any = new Data({
        format: 'f32-planar',
        sampleRate: buffer.sampleRate,
        numberOfFrames: frames,
        numberOfChannels: channels,
        timestamp: Math.round(offset / buffer.sampleRate * 1e6),
        data: planar,
      });
      try { encoder.encode(data); } finally { data.close(); }
      // Wait for actual capacity instead of polling once after four milliseconds.
      // This also bounds a slow encoder to at most 17 queued audio packets.
      while (encoder.encodeQueueSize > 16 && !encodeError && encoder.state !== 'closed') {
        await new Promise<void>(resolve => {
          const stop = () => {
            encoder.removeEventListener('dequeue', check);
            if (capacity.wake === check) capacity.wake = undefined;
            if (capacity.stop === stop) capacity.stop = undefined;
            resolve();
          };
          const check = () => {
            if (encoder.encodeQueueSize <= 16 || encodeError || encoder.state === 'closed') stop();
          };
          capacity.wake = check; capacity.stop = stop;
          encoder.addEventListener('dequeue', check);
          // Capacity may have changed immediately before listener registration.
          check();
        });
      }
    }
    await encoder.flush();
    if (encodeError || !chunks.length) return null;
    return { chunks, priv: privateData || opusHead(channels, buffer.sampleRate), rate: buffer.sampleRate, channels };
  } finally {
    capacity.stop?.();
    if (encoder.state !== 'closed') encoder.close();
  }
}

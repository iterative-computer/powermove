import { expect, test } from './helpers/app';

test('reports the hidden packaged-renderer video delivery capabilities', async ({ session }) => {
  const capabilities = await session.page.evaluate(async () => {
    const encoder = (window as any).VideoEncoder;
    const h264 = encoder?.isConfigSupported
      ? await encoder.isConfigSupported({
        codec: 'avc1.42001f',
        width: 64,
        height: 64,
        bitrate: 1_000_000,
        framerate: 30,
        avc: { format: 'avc' },
      }).catch(() => null)
      : null;
    const recorderTypes = [
      'video/mp4;codecs=avc1.42001f,mp4a.40.2',
      'video/mp4;codecs=avc1.42001f',
      'video/mp4',
      'video/webm;codecs=vp9,opus',
    ];
    return {
      h264: Boolean(h264?.supported),
      recorder: Object.fromEntries(recorderTypes.map((type) => [
        type,
        Boolean((window as any).MediaRecorder?.isTypeSupported?.(type)),
      ])),
    };
  });

  console.log(`export capabilities: ${JSON.stringify(capabilities)}`);
  expect(capabilities.recorder['video/webm;codecs=vp9,opus']).toBe(true);
});

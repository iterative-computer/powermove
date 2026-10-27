import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterAll, describe, expect, it } from 'vitest';
import { mp4VideoSamples } from './mp4-samples';

const ffmpeg = path.resolve('node_modules/ffmpeg-static/ffmpeg');
const fixtures = path.resolve('e2e/fixtures');
const scratch = mkdtempSync(path.join(tmpdir(), 'pm-mp4-samples-'));
afterAll(() => rmSync(scratch, { recursive: true, force: true }));

/** FFmpeg's own demuxed packets (decode order), after its edit-list handling:
 * the timeline Chromium's FFmpeg-based `<video>` plays. */
function packets(file: string) {
  const text = execFileSync(ffmpeg, ['-v', 'error', '-copyts', '-i', file, '-map', '0:v:0', '-c', 'copy', '-f', 'framecrc', '-'], { encoding: 'utf8' });
  const base = /#tb 0: (\d+)\/(\d+)/.exec(text)!;
  return text.split('\n').filter(line => /^0,/.test(line)).map(line => {
    const [, , pts, , size] = line.split(',').map(value => value.trim());
    return { pts: Number(pts) * Number(base[1]) / Number(base[2]), size: Number(size) };
  });
}
async function read(file: string) {
  return mp4VideoSamples(new Blob([readFileSync(file)]));
}
function encode(name: string, ...args: string[]) {
  const file = path.join(scratch, name);
  execFileSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=96x64:rate=30', '-t', '2', ...args, '-y', file]);
  return file;
}
async function expectSameTimeline(file: string) {
  const samples = (await read(file))!;
  expect(samples).toBeDefined();
  const expected = packets(file);
  expect(samples.sizes.length).toBe(expected.length);
  expected.forEach((packet, i) => {
    expect(samples.sizes[i]).toBe(packet.size);
    expect(samples.pts[i]).toBeCloseTo(packet.pts, 9);
  });
  const bytes = readFileSync(file);
  // The first sample begins with a length-prefixed NAL/OBU, not container data.
  expect(samples.offsets[0]! + samples.sizes[0]!).toBeLessThanOrEqual(bytes.length);
  return samples;
}

describe('MP4 video sample tables', () => {
  it.each(['h264-aac.mp4', 'h264-aac.mov', 'hevc.mp4', 'hevc.mov', 'av1.mp4'])('matches FFmpeg packet timing for %s', async name => {
    const samples = await expectSameTimeline(path.join(fixtures, name));
    expect(samples.keys[0]).toBe(1);
    expect(samples.width).toBe(160);
    expect(samples.height).toBe(90);
  });

  it('describes decoder configurations Chromium accepts', async () => {
    const h264 = (await read(path.join(fixtures, 'h264-aac.mp4')))!;
    expect(h264.codec).toMatch(/^avc1\.[0-9a-f]{6}$/);
    expect(h264.description?.[0]).toBe(1);
    expect((await read(path.join(fixtures, 'hevc.mp4')))!.codec).toMatch(/^hvc1\.1\.6\.L\d+(\.[0-9A-F]+)*$/);
    expect((await read(path.join(fixtures, 'av1.mp4')))!.codec).toMatch(/^av01\.0\.\d\dM\.08$/);
  });

  it('applies B-frame composition offsets and the edit list like FFmpeg', async () => {
    const edited = await expectSameTimeline(encode('bframes.mp4', '-c:v', 'libx264', '-bf', '3', '-g', '12', '-pix_fmt', 'yuv420p'));
    expect(Math.min(...edited.pts)).toBeCloseTo(0, 9);
    expect([...edited.pts].some((pts, i) => i && pts < edited.pts[i - 1]!)).toBe(true);
    expect(edited.keys.filter(Boolean).length).toBeGreaterThan(1);
    const unedited = await expectSameTimeline(encode('bframes-no-edit.mov', '-c:v', 'libx264', '-bf', '2', '-pix_fmt', 'yuv420p', '-use_editlist', '0'));
    expect(Math.min(...unedited.pts)).toBeGreaterThan(0);
  });

  it('declines layouts an in-order decode cannot reproduce', async () => {
    expect(await read(encode('fragmented.mp4', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-movflags', 'frag_keyframe+empty_moov'))).toBeUndefined();
    const upright = encode('upright.mp4', '-c:v', 'libx264', '-pix_fmt', 'yuv420p'), rotated = path.join(scratch, 'rotated.mp4');
    execFileSync(ffmpeg, ['-v', 'error', '-display_rotation', '90', '-i', upright, '-c', 'copy', '-y', rotated]);
    expect(await read(upright)).toBeDefined();
    expect(await read(rotated)).toBeUndefined();
    expect(await read(encode('anamorphic.mp4', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-vf', 'setsar=4/3'))).toBeUndefined();
    expect(await read(path.join(fixtures, 'vp9-opus.webm'))).toBeUndefined();
    expect(await mp4VideoSamples(new Blob(['not a video']))).toBeUndefined();
  });
});

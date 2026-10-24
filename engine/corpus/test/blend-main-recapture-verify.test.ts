import { expect, test } from 'bun:test';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { sha256, writeFile, writeFloat, writePng } from '../src/output';
import { verifyCapture } from '../src/verify';

const reason = 'recaptured after main 2b1fa379a blend change';
const mergedMainHead = 'a'.repeat(40), appBuildSha256 = 'b'.repeat(64);

function fixture() {
  const root = mkdtempSync(path.join(os.tmpdir(), 'pm-blend1-verify-'));
  try {
    const width = 2, height = 2, time = .5;
    const directory = 'projects/composite/blend-modes-alpha';
    const loaded = writeFile(root, `${directory}/loaded.json`, '{"id":"Ptest"}');
    const oldOpaque = Uint8Array.from({ length: 16 }, (_, i) => i % 4 === 3 ? 255 : 40 + i);
    const newOpaque = Uint8Array.from(oldOpaque); newOpaque[0] = newOpaque[0]! + 3;
    const originalPng = writePng(root, `${directory}/old.png`, width, height, oldOpaque, {});
    const correctedPng = writePng(root, `${directory}/new.png`, width, height, newOpaque, {});
    const oldFloat = Buffer.from(new Float32Array([.1, .2, .3, 1, .2, .3, .4, 1, .3, .4, .5, 1, .4, .5, .6, 1]).buffer);
    const newFloat = Buffer.from(oldFloat);
    newFloat.writeFloatLE(.25, 0);
    const header = { width, height, time, rowOrder: 'top-down' as const, alpha: 'premultiplied' as const,
      sourcePrecision: 'binary16' as const, encoding: 'E', source: 'BLEND-1 fixture' };
    const originalFloat = writeFloat(root, `${directory}/old.rgba32f`, header, oldFloat);
    const correctedFloat = writeFloat(root, `${directory}/new.rgba32f`, header, newFloat);
    const kinds = {
      opaque: { original: originalPng, corrected: correctedPng, old: oldOpaque, current: newOpaque },
      float: { original: originalFloat, corrected: correctedFloat, old: oldFloat, current: newFloat },
    };
    const blendMainRecapture: Record<string, any> = {};
    for (const [kind, value] of Object.entries(kinds)) {
      const historical = writeFile(root, `${directory}/${kind}.old.raw`, value.old);
      const oldCaptureSha256 = sha256(value.old), currentCaptureSha256 = sha256(value.current);
      const evidence = writeFile(root, `${directory}/${kind}.evidence.json`, JSON.stringify({
        id: 'BLEND-1', mainCommit: '2b1fa379a', reason, kind, projectId: 'composite/blend-modes-alpha', time,
        original: value.original, current: value.corrected, historical,
        oldCaptureSha256, originalPixelSha256: oldCaptureSha256, currentCaptureSha256,
        loadedDocumentSha256: loaded.sha256, oldExact: true, currentRepeatExact: true,
        mergedMainHead, appBuild: { sha256: appBuildSha256 },
        equation: {
          old: ['vec3 res = mix(dc, bc, s.a);', 'o = vec4(res * a, a);'],
          current: ['vec3 res = (1. - s.a) * d.rgb + (1. - d.a) * s.rgb + s.a * d.a * bc;', 'o = vec4(res, a);'],
        },
        setting: { useNewEngine: false, colorDepth: 16 },
      }));
      blendMainRecapture[kind] = { id: 'BLEND-1', mainCommit: '2b1fa379a', reason,
        original: value.original, corrected: value.corrected, historical, evidence,
        oldCaptureSha256, currentCaptureSha256, mergedMainHead, appBuildSha256 };
    }
    const sample = { time, width, height, png: correctedPng, float: correctedFloat, blendMainRecapture };
    const project = { id: 'composite/blend-modes-alpha', dir: directory, loaded, samples: [sample] };
    const local = structuredClone(project);
    const manifest = { format: 'powermove-engine-corpus', version: 1, projects: [project],
      totals: { projects: 1, frames: 1, floatFrames: 1, evalDumps: 0, failedSamples: 0,
        bytes: correctedPng.bytes + correctedFloat.data.bytes + correctedFloat.header.bytes } };
    const save = () => {
      writeFile(root, `${directory}/project.json`, JSON.stringify(local));
      writeFile(root, 'manifest.json', JSON.stringify(manifest));
    };
    save();
    return { root, project, local, manifest, save };
  } catch (error) { rmSync(root, { recursive: true, force: true }); throw error; }
}

test('BLEND-1 verifies retained PNG, float, historical raw, evidence, and active records', () => {
  const f = fixture();
  try {
    expect(verifyCapture(f.root).problems).toEqual([]);
    expect(verifyCapture(f.root).unlisted).toEqual([]);
  } finally { rmSync(f.root, { recursive: true, force: true }); }
});

test('BLEND-1 rejects a changed historical raw capture', () => {
  const f = fixture();
  try {
    const file = f.project.samples[0]!.blendMainRecapture.opaque.historical.file;
    writeFileSync(path.join(f.root, file), 'tampered');
    expect(verifyCapture(f.root).problems.some(p => p.includes('historical') && p.includes('SHA-256'))).toBe(true);
  } finally { rmSync(f.root, { recursive: true, force: true }); }
});

test('BLEND-1 rejects inconsistent project and manifest metadata', () => {
  const f = fixture();
  try {
    f.local.samples[0]!.blendMainRecapture.float.mainCommit = 'other';
    f.save();
    expect(verifyCapture(f.root).problems.some(p => p.includes('metadata differs'))).toBe(true);
  } finally { rmSync(f.root, { recursive: true, force: true }); }
});

test('BLEND-1 rejects evidence with a valid file hash but false proof', () => {
  const f = fixture();
  try {
    const a = f.project.samples[0]!.blendMainRecapture.opaque;
    const evidence = writeFile(f.root, a.evidence.file, JSON.stringify({
      ...JSON.parse(readFileSync(path.join(f.root, a.evidence.file), 'utf8')), oldExact: false,
    }));
    a.evidence = evidence;
    f.local.samples[0]!.blendMainRecapture.opaque.evidence = evidence;
    f.save();
    expect(verifyCapture(f.root).problems.some(p => p.includes('evidence differs'))).toBe(true);
  } finally { rmSync(f.root, { recursive: true, force: true }); }
});

test('BLEND-1 rejects a retained record outside the capture', () => {
  const f = fixture();
  try {
    for (const project of [f.project, f.local]) project.samples[0]!.blendMainRecapture.float.historical.file = '../outside.raw';
    f.save();
    expect(verifyCapture(f.root).problems.some(p => p.includes('outside the capture'))).toBe(true);
  } finally { rmSync(f.root, { recursive: true, force: true }); }
});

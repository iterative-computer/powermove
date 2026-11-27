import { expect, test } from 'bun:test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { writeFile, writePng } from '../src/output';
import { verifyCapture } from '../src/verify';

function fixture() {
  const root = mkdtempSync(path.join(os.tmpdir(), 'pm-compclose1-'));
  try {
    const rgba = new Uint8Array(480 * 270 * 4).fill(255);
    const original = writePng(root, 'original.png', 480, 270, rgba, {});
    rgba[0] = 0;
    const corrected = writePng(root, 'corrected.png', 480, 270, rgba, {});
    const proof = writeFile(root, 'evidence.json', '{"fixture":true}');
    const project = { id: 'motion/blur-video', dir: 'projects/motion/blur-video', samples: [.5, 1.2].map(time => ({
      time, width: 480, height: 270, png: original,
      referenceSubstitution: { id: 'COMPCLOSE-1', ownerDecision: '2026-10-19', reason: 'Owner-authorized center upload', original, corrected, evidence: [path.join(root, proof.file)] },
    })) };
    writeFile(root, project.dir + '/project.json', JSON.stringify(project));
    const manifest = { format: 'powermove-engine-corpus', version: 1, projects: [project], totals: { projects: 1, frames: 2, floatFrames: 0, evalDumps: 0, failedSamples: 0, bytes: original.bytes * 2 } };
    const save = () => writeFile(root, 'manifest.json', JSON.stringify(manifest));
    save();
    return { root, project, manifest, save };
  } catch (error) { rmSync(root, { recursive: true, force: true }); throw error; }
}

test('COMPCLOSE-1 verifies both references and evidence without changing original totals', () => {
  const f = fixture();
  try { expect(verifyCapture(f.root).problems).toEqual([]); expect(verifyCapture(f.root).unlisted).toEqual([]); }
  finally { rmSync(f.root, { recursive: true, force: true }); }
});

test('COMPCLOSE-1 never authorizes another project, time, or decision', () => {
  for (const mutate of [
    (f: ReturnType<typeof fixture>) => { f.project.id = 'motion/other'; },
    (f: ReturnType<typeof fixture>) => { f.project.samples[0]!.time = .6; },
    (f: ReturnType<typeof fixture>) => { f.project.samples[0]!.referenceSubstitution.id = 'OTHER'; },
  ]) {
    const f = fixture();
    try { mutate(f); f.save(); expect(verifyCapture(f.root).problems.some(p => p.includes('scope'))).toBe(true); }
    finally { rmSync(f.root, { recursive: true, force: true }); }
  }
});

test('COMPCLOSE-1 rejects changed corrected pixels and missing evidence', () => {
  const f = fixture();
  try {
    writeFileSync(path.join(f.root, 'corrected.png'), 'tampered');
    rmSync(path.join(f.root, 'evidence.json'));
    const problems = verifyCapture(f.root).problems;
    expect(problems.some(p => p.includes('SHA-256'))).toBe(true);
    expect(problems.some(p => p.includes('evidence'))).toBe(true);
  } finally { rmSync(f.root, { recursive: true, force: true }); }
});

test('COMPCLOSE-1 annotations must agree in the project and manifest', () => {
  const f = fixture();
  try {
    f.project.samples[0]!.referenceSubstitution.reason = 'Changed only in manifest';
    f.save();
    expect(verifyCapture(f.root).problems.some(p => p.includes('metadata differs'))).toBe(true);
  } finally { rmSync(f.root, { recursive: true, force: true }); }
});

test('COMPCLOSE-1 evidence must live in the capture, not a temporary directory', () => {
  const f = fixture();
  const outside = mkdtempSync(path.join(os.tmpdir(), 'pm-compclose1-evidence-'));
  try {
    const proof = path.join(outside, 'comparison.json');
    writeFileSync(proof, '{"fixture":true}');
    for (const sample of f.project.samples) sample.referenceSubstitution.evidence.push(proof);
    writeFile(f.root, f.project.dir + '/project.json', JSON.stringify(f.project));
    f.save();
    expect(verifyCapture(f.root).problems.filter(p => p.includes('outside the capture')).length).toBe(2);
  } finally {
    rmSync(f.root, { recursive: true, force: true });
    rmSync(outside, { recursive: true, force: true });
  }
});

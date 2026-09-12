import { describe, expect, it } from 'vitest';
import { compactEditLog, MAX_EDIT_LOG_ENTRY_BYTES, MAX_EDIT_LOG_PAYLOAD_BYTES } from './edit-log';
import { sanitizeProject } from './validate/project';

describe('edit log payload budget', () => {
  it('retains provenance while omitting oversized operations and structural snapshots', () => {
    const entry = { id: 'bulk', label: 'Cut out subject', summary: ['Updated subject track'],
      operations: [{ path: 'masks', value: 'x'.repeat(MAX_EDIT_LOG_ENTRY_BYTES) }], structural: { before: 'large' } };
    const [compacted] = compactEditLog([entry]);
    expect(compacted).toEqual({ id: 'bulk', label: entry.label, summary: entry.summary, operations: [], payloadOmitted: true });
    expect(entry.operations).toHaveLength(1);
    const project = sanitizeProject({ edits: [entry] });
    expect(project.edits[0]?.payloadOmitted).toBe(true);
    expect(project.edits[0]?.operations).toEqual([]);
    expect(sanitizeProject(project).edits).toEqual(project.edits);
  });

  it('keeps newest payloads within the total budget and retains older summaries', () => {
    const entries = Array.from({ length: 30 }, (_, i) => ({ id: String(i), summary: [`Edit ${i}`], operations: [{ value: 'x'.repeat(100_000) }] }));
    const compacted = compactEditLog(entries);
    expect(compacted).toHaveLength(30);
    expect(compacted[0]?.operations).toEqual([]);
    expect(compacted.at(-1)).toBe(entries.at(-1));
    expect(new TextEncoder().encode(JSON.stringify(compacted.map(e => e.operations))).byteLength).toBeLessThan(MAX_EDIT_LOG_PAYLOAD_BYTES);
    expect(compactEditLog(compacted)).toEqual(compacted);
  });
});

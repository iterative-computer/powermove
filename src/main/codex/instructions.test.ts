import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { AGENT_COMMAND_TYPES, EDIT_COMMAND_TYPES } from '../../shared/edit-vocabulary';
import { agentInstructions, agentResultSchema } from './instructions';

function normalizeWhitespace(value: string): string {
  return value.replace(/\s+/gu, ' ').trim();
}

describe('autonomous agent contract', () => {
  it('keeps all 18 edit commands reachable by the agent', () => {
    expect(EDIT_COMMAND_TYPES).toHaveLength(18);
    expect(new Set(EDIT_COMMAND_TYPES).size).toBe(18);
    expect(AGENT_COMMAND_TYPES).toEqual(EDIT_COMMAND_TYPES);
  });

  it('matches the normalized Swift instruction golden with template values substituted', async () => {
    const goldenPath = path.join(process.cwd(), 'tests/fixtures/agent-instructions.golden.txt');
    const golden = (await readFile(goldenPath, 'utf8'))
      .replace('\\(artifactRelativePath)', 'artifacts/run-123')
      .replace('\\(access)', 'project')
      .replace('\\(projectName)', 'Golden Project');

    expect(
      normalizeWhitespace(
        agentInstructions({
          projectName: 'Golden Project',
          artifactPath: 'artifacts/run-123',
          access: 'project'
        })
      )
    ).toBe(normalizeWhitespace(golden));
  });

  it('matches the Swift result-schema golden', async () => {
    const goldenPath = path.join(process.cwd(), 'tests/fixtures/agent-result-schema.golden.json');
    const golden = JSON.parse(await readFile(goldenPath, 'utf8')) as unknown;
    expect(agentResultSchema()).toEqual(golden);
  });
});

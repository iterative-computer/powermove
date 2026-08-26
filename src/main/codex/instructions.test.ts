import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { AGENT_COMMAND_TYPES, EDIT_COMMAND_TYPES } from '../../shared/edit-vocabulary';
import { agentInstructions, agentResultSchema, buildFixPrompt } from './instructions';

function normalizeText(value: string): string {
  return value
    .replace(/\r\n?/gu, '\n')
    .split('\n')
    .map((line) => line.replace(/[ \t]+$/gu, ''))
    .join('\n')
    .trim();
}

describe('autonomous agent contract', () => {
  it('keeps every distinct edit command reachable by the agent', () => {
    expect(EDIT_COMMAND_TYPES).toHaveLength(19);
    expect(new Set(EDIT_COMMAND_TYPES).size).toBe(19);
    expect(AGENT_COMMAND_TYPES).toEqual(EDIT_COMMAND_TYPES);
  });

  it('matches the committed instruction golden with template values substituted', async () => {
    const goldenPath = path.join(process.cwd(), 'tests-vitest/fixtures/agent-instructions.golden.txt');
    const golden = (await readFile(goldenPath, 'utf8'))
      .replace('\\(artifactRelativePath)', 'artifacts/run-123')
      .replace('\\(access)', 'project')
      .replace('\\(extensionsDir)', '/Users/test/Library/Application Support/Powermove/extensions')
      .replace('\\(projectName)', 'Golden Project');

    expect(
      normalizeText(
        agentInstructions({
          projectName: 'Golden Project',
          artifactPath: 'artifacts/run-123',
          access: 'project',
          extensionsDir: '/Users/test/Library/Application Support/Powermove/extensions'
        })
      )
    ).toBe(normalizeText(golden));
  });

  it('matches the committed result-schema golden', async () => {
    const goldenPath = path.join(process.cwd(), 'tests-vitest/fixtures/agent-result-schema.golden.json');
    const golden = JSON.parse(await readFile(goldenPath, 'utf8')) as unknown;
    expect(agentResultSchema()).toEqual(golden);
  });

  it('gives the agent the extension directory, API pack, and extension strategy', () => {
    const instructions = agentInstructions({
      projectName: 'Project',
      artifactPath: 'artifacts/run-1',
      access: 'project',
      extensionsDir: '/absolute/user/extensions'
    });

    expect(instructions).toContain('EXTENDING POWERMOVE');
    expect(instructions).toContain('/absolute/user/extensions');
    expect(instructions).toContain('powermove-api/EXTENSIONS.md');
    expect(instructions).toMatch(/contribute.+override.+fork.+replaces/s);
    expect(instructions).toContain('folder name must equal the extension manifest id');
    expect(instructions).toContain('Never edit the app bundle');
    expect(instructions).toContain("result's extensions array");
  });

  it('keeps scene edits on typed commands and stays below 6 KB', () => {
    const instructions = agentInstructions({
      projectName: 'Project',
      artifactPath: 'artifacts/run-1',
      access: 'computer',
      extensionsDir: '/absolute/user/extensions'
    });

    expect(instructions).toMatch(/scene edits.+return typed commands/s);
    expect(instructions).toContain('Return each command as one JSON-encoded string');
    expect(instructions).toContain('externalActions');
    expect(Buffer.byteLength(instructions, 'utf8')).toBeLessThanOrEqual(6 * 1024);
  });

  it('defines optional, bounded extension changes in the result schema', () => {
    const schema = agentResultSchema() as {
      required: string[];
      properties: { extensions: Record<string, unknown> };
    };
    const extensions = schema.properties.extensions as {
      maxItems: number;
      items: { required: string[]; properties: Record<string, Record<string, unknown>> };
    };

    expect(schema.required).not.toContain('extensions');
    expect(extensions.maxItems).toBe(32);
    expect(extensions.items.required).toEqual(['id', 'action']);
    expect(extensions.items.properties.id?.pattern).toBe('^[a-z0-9][a-z0-9-]{1,63}$');
    expect(extensions.items.properties.action?.enum).toEqual(['created', 'updated', 'removed']);
    expect(extensions.items.properties).toHaveProperty('summary');
  });
});

describe('buildFixPrompt', () => {
  it('renders the extension failure and all supplied source files', () => {
    const prompt = buildFixPrompt({
      id: 'broken-extension',
      error: 'index.ts:4: missing semicolon',
      files: [
        { path: 'manifest.json', text: '{"id":"broken-extension"}' },
        { path: 'index.ts', text: 'export default function activate() {}' }
      ]
    });

    expect(prompt).toContain('The extension `broken-extension` fails: index.ts:4: missing semicolon. Files:');
    expect(prompt).toContain('<file path="manifest.json">\n{"id":"broken-extension"}\n</file>');
    expect(prompt).toContain('<file path="index.ts">\nexport default function activate() {}\n</file>');
  });

  it('states when no source files were supplied', () => {
    expect(buildFixPrompt({ id: 'broken-extension', error: 'boom', files: [] }))
      .toBe('The extension `broken-extension` fails: boom. Files:\n\n(none)');
  });
});

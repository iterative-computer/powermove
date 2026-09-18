import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { AGENT_COMMAND_TYPES, EDIT_COMMAND_TYPES } from '../../shared/edit-vocabulary';
import { AGENT_RESPONSE_STYLE } from '../../shared/response-style';
import { agentInstructions, agentResultSchema, buildFixPrompt, buildRebasePrompt } from './instructions';

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
    expect(EDIT_COMMAND_TYPES).toHaveLength(22);
    expect(new Set(EDIT_COMMAND_TYPES).size).toBe(22);
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
    expect(instructions).toMatch(/contribute.+override.+fork_builtin_extension/s);
    expect(instructions).toContain('folder name must equal the extension manifest id');
    expect(instructions).toContain('Never edit the app bundle');
    expect(instructions).toContain("result's extensions array");
  });

  it('keeps scene edits on typed commands and stays below 9 KB', () => {
    const instructions = agentInstructions({
      projectName: 'Project',
      artifactPath: 'artifacts/run-1',
      access: 'computer',
      extensionsDir: '/absolute/user/extensions'
    });

    expect(instructions).toMatch(/scene edits.+return typed commands/s);
    expect(instructions).toContain('Return each command as one JSON-encoded string');
    expect(instructions).toContain('externalActions');
    expect(instructions).toContain('Never open or show a new window for testing');
    expect(instructions).toContain('source checkout, package scripts, and Electron test harness are not available');
    expect(instructions).not.toMatch(/src\/extensions|npm run|bun run/u);
    expect(instructions).toContain('isolated temporary test data');
    expect(Buffer.byteLength(instructions, 'utf8')).toBeLessThanOrEqual(9 * 1024);
  });

  it('holds every user-visible field to the shared response style', () => {
    const instructions = agentInstructions({
      projectName: 'Project',
      artifactPath: 'artifacts/run-1',
      access: 'project',
      extensionsDir: '/absolute/user/extensions'
    });

    expect(instructions).toContain(AGENT_RESPONSE_STYLE);
    expect(instructions).toContain('summary is one to three sentences');
    expect(instructions).toContain('Each note is one line');
    // Density may never cost a fact; the panel is narrow, not forgetful.
    expect(AGENT_RESPONSE_STYLE).toContain('Shorten, never omit');
  });

  it('makes editable values keyframeable by default', () => {
    const instructions = agentInstructions({
      projectName: 'Project',
      artifactPath: 'artifacts/run-1',
      access: 'project',
      extensionsDir: '/absolute/user/extensions'
    });

    expect(instructions).toContain('ANIMATION-FIRST VALUES');
    expect(instructions).toContain('every user-editable project value as keyframeable by default');
    expect(instructions).toMatch(/effect, layer type, generated control, or extension/);
    expect(instructions).toMatch(/real editable property\/keyframe model.+set_property or replace_keyframes/s);
    expect(instructions).toContain('Never flatten adjustable values or duplicate state');
    expect(instructions).toContain('preserveHandEdits: false');
    expect(instructions).toContain('only for explicitly requested hand-edited channel changes');
  });

  it('defines required, bounded extension changes in the strict result schema', () => {
    const schema = agentResultSchema() as {
      required: string[];
      properties: { extensions: Record<string, unknown> };
    };
    const extensions = schema.properties.extensions as {
      maxItems: number;
      items: { required: string[]; properties: Record<string, Record<string, unknown>> };
    };

    expect(schema.required).toContain('extensions');
    expect(extensions.maxItems).toBe(32);
    expect(extensions.items.required).toEqual(['id', 'action', 'summary']);
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

describe('buildRebasePrompt', () => {
  it('matches the committed rebase prompt contract', () => {
    expect(buildRebasePrompt({
      forkId: 'my-timeline',
      forkedFrom: 'timeline',
      base: '1.2.0',
      current: '1.4.0'
    })).toBe(`Update the user fork \`my-timeline\`. It was forked from \`timeline@1.2.0\`, and Powermove now ships \`timeline@1.4.0\`.

Call \`stage_fork_rebase\` with {"id":"my-timeline"}. Work only inside the returned staging paths. Merge every file in \`changedUpstream\` into \`workingDir\` using a three-way comparison: \`baseDir\` is the old base, the working copy is the user's version (theirs), and \`oursDir\` is the shipped version (ours). Preserve the user's changes, behavior, and intent. Treat every path in \`conflicts\` with care, and explain each conflict resolution in your final response. When finished, return the fork in the result's \`extensions\` array as {"id":"my-timeline","action":"updated","summary":"..."}.`);
  });

  it('stays below the 1.5 KB prompt budget', () => {
    const prompt = buildRebasePrompt({
      forkId: 'x'.repeat(64),
      forkedFrom: 'y'.repeat(64),
      base: '999999.999999.999999',
      current: '999999.999999.999999'
    });
    expect(Buffer.byteLength(prompt, 'utf8')).toBeLessThan(1_500);
  });
});

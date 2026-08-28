import { chmod, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { AGENT_TESTING_INSTRUCTIONS } from '../../shared/agent-testing';
import {
  ADAPTER_VERSION,
  REQUIRED_CODEX_FLAGS,
  buildAutonomousArgv,
  buildEditorArgv,
  capabilities
} from './adapter';

describe('Codex CLI adapter', () => {
  it('builds the exact editor argv with the prompt before every image', () => {
    expect(
      buildEditorArgv({
        schemaPath: '/tmp/editor/schema.json',
        outputPath: '/tmp/editor/result.json',
        prompt: 'Polish this composition',
        imagePaths: ['/tmp/editor/frame-0.png', '/tmp/editor/frame-1.jpg'],
        model: 'gpt-5-codex',
        reasoningEffort: 'high',
        disabledSkillPaths: ['/Users/test/.agents/skills/custom/SKILL.md']
      })
    ).toEqual([
      'exec',
      '--ephemeral',
      '--skip-git-repo-check',
      '--ignore-user-config',
      '--ignore-rules',
      '--disable',
      'plugins',
      '--disable',
      'apps',
      '--disable',
      'skill_search',
      '--disable',
      'skill_mcp_dependency_install',
      '--config',
      'mcp_servers={}',
      '--config',
      'skills.include_instructions=false',
      '--config',
      'skills.bundled.enabled=false',
      '--config',
      'skills.config=[{path="/Users/test/.agents/skills/custom/SKILL.md",enabled=false}]',
      '--sandbox',
      'read-only',
      '--output-schema',
      '/tmp/editor/schema.json',
      '--output-last-message',
      '/tmp/editor/result.json',
      '--json',
      '--model',
      'gpt-5-codex',
      '--config',
      'model_reasoning_effort="high"',
      `${AGENT_TESTING_INSTRUCTIONS}\n\nPolish this composition`,
      '--image',
      '/tmp/editor/frame-0.png',
      '--image',
      '/tmp/editor/frame-1.jpg'
    ]);
  });

  it('builds the exact fresh project-authority autonomous argv', () => {
    expect(
      buildAutonomousArgv({
        schemaPath: '/workspace/.powermove/result-schema.json',
        outputPath: '/workspace/.powermove/result-run-1.json',
        prompt: 'Make a launch trailer',
        imagePaths: ['/workspace/inputs/references/reference-0.png'],
        model: null,
        reasoningEffort: null,
        access: 'project',
        extensionsDir: '/user-data/extensions',
        sessionId: null,
        instructions: 'AGENT INSTRUCTIONS',
        disabledSkillPaths: []
      })
    ).toEqual([
      '--search',
      '--approve-for-me',
      '--add-dir',
      '/user-data/extensions',
      'exec',
      '--ignore-user-config',
      '--skip-git-repo-check',
      '--disable',
      'plugins',
      '--disable',
      'apps',
      '--disable',
      'skill_search',
      '--disable',
      'skill_mcp_dependency_install',
      '--config',
      'mcp_servers={}',
      '--config',
      'skills.include_instructions=false',
      '--config',
      'skills.bundled.enabled=false',
      '--output-schema',
      '/workspace/.powermove/result-schema.json',
      '--output-last-message',
      '/workspace/.powermove/result-run-1.json',
      '--json',
      'AGENT INSTRUCTIONS\n\nUSER REQUEST\nMake a launch trailer',
      '--image',
      '/workspace/inputs/references/reference-0.png'
    ]);
  });

  it('builds the exact resumed computer-authority autonomous argv', () => {
    expect(
      buildAutonomousArgv({
        schemaPath: '/workspace/.powermove/result-schema.json',
        outputPath: '/workspace/.powermove/result-run-2.json',
        prompt: 'Publish the approved deliverable',
        imagePaths: [],
        model: 'gpt-5-codex',
        reasoningEffort: 'medium',
        access: 'computer',
        extensionsDir: '/user-data/extensions',
        sessionId: ' thread-123\n',
        instructions: 'AGENT INSTRUCTIONS',
        disabledSkillPaths: []
      })
    ).toEqual([
      '--search',
      '--dangerously-bypass-approvals-and-sandbox',
      '--add-dir',
      '/user-data/extensions',
      'exec',
      'resume',
      '--ignore-user-config',
      '--skip-git-repo-check',
      '--disable',
      'plugins',
      '--disable',
      'apps',
      '--disable',
      'skill_search',
      '--disable',
      'skill_mcp_dependency_install',
      '--config',
      'mcp_servers={}',
      '--config',
      'skills.include_instructions=false',
      '--config',
      'skills.bundled.enabled=false',
      '--output-schema',
      '/workspace/.powermove/result-schema.json',
      '--output-last-message',
      '/workspace/.powermove/result-run-2.json',
      '--json',
      '--model',
      'gpt-5-codex',
      '--config',
      'model_reasoning_effort="medium"',
      'thread-123',
      'AGENT INSTRUCTIONS\n\nUSER REQUEST\nPublish the approved deliverable'
    ]);
  });

  it('isolates every run from the user config while retaining Codex auth', () => {
    const argv = buildAutonomousArgv({
      schemaPath: '/workspace/.powermove/result-schema.json',
      outputPath: '/workspace/.powermove/result-run-3.json',
      prompt: 'Retry without integrations',
      imagePaths: [],
      model: null,
      reasoningEffort: null,
      access: 'project',
      extensionsDir: '/user-data/extensions',
      sessionId: null,
      instructions: 'AGENT INSTRUCTIONS',
      disabledSkillPaths: []
    });
    expect(argv.slice(argv.indexOf('exec'), argv.indexOf('exec') + 3)).toEqual([
      'exec',
      '--ignore-user-config',
      '--skip-git-repo-check'
    ]);
    expect(argv.filter(flag => flag === '--ignore-user-config')).toHaveLength(1);
    expect(argv).toContain('mcp_servers={}');
  });

  it('reports supported and missing flags from codex exec --help', async () => {
    const directory = await mkdtemp(path.join(tmpdir(), 'powermove-adapter-'));
    const binary = path.join(directory, 'codex');
    const shown = REQUIRED_CODEX_FLAGS.filter((_, index) => index % 2 === 0);
    await writeFile(
      binary,
      `#!/bin/sh\n[ "$1 $2" = "exec --help" ] || exit 9\nprintf '%s\\n' '${shown.join(' ')}'\n`,
      'utf8'
    );
    await chmod(binary, 0o755);

    const result = await capabilities(binary);
    expect(result).toEqual({
      adapterVersion: ADAPTER_VERSION,
      supported: [...shown],
      missing: REQUIRED_CODEX_FLAGS.filter((flag) => !shown.includes(flag))
    });
  });
});

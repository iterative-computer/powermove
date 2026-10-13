import { chmod, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { AGENT_TESTING_INSTRUCTIONS } from '../../shared/agent-testing';
import {
  ADAPTER_VERSION,
  REQUIRED_CODEX_FLAGS,
  buildAutonomousArgv,
  PROJECT_PERMISSION_PROFILE,
  buildEditorArgv,
  capabilities
} from './adapter';

describe('Codex CLI adapter', () => {
  it('makes configured external tools available in the editor CLI path', () => {
    const argv = buildEditorArgv({
      schemaPath: '/tmp/schema.json', outputPath: '/tmp/result.json', prompt: 'Make audio',
      imagePaths: [], model: null, reasoningEffort: null,
      externalMcpServers: { studio: { command: 'fixture-server' } }
    });
    expect(argv).toContain('mcp_servers."studio"={"command"="fixture-server"}');
  });

  it('builds the exact editor argv with the prompt before every image', () => {
    expect(
      buildEditorArgv({
        schemaPath: '/tmp/editor/schema.json',
        outputPath: '/tmp/editor/result.json',
        prompt: 'Polish this composition',
        imagePaths: ['/tmp/editor/frame-0.png', '/tmp/editor/frame-1.jpg'],
        model: 'gpt-5-codex',
        reasoningEffort: 'high',
      })
    ).toEqual([
      'exec',
      '--ephemeral',
      '--skip-git-repo-check',
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
        shellNetwork: true,
        extensionsDir: '/user-data/extensions',
        sessionId: null,
        instructions: 'AGENT INSTRUCTIONS',
      })
    ).toEqual([
      '--search',
      '--add-dir',
      '/user-data/extensions',
      'exec',
      '--skip-git-repo-check',
      '--config', 'approvals_reviewer="auto_review"',
      '--config', 'approval_policy="on-request"',
      '--config', 'default_permissions="powermove"',
      '--config', 'permissions.powermove.extends=":workspace"',
      '--config', 'permissions.powermove.network.enabled=true',
      '--output-schema',
      '/workspace/.powermove/result-schema.json',
      '--output-last-message',
      '/workspace/.powermove/result-run-1.json',
      '--json',
      '--image',
      '/workspace/inputs/references/reference-0.png',
      '--',
      'AGENT INSTRUCTIONS\n\nSHELL NETWORK\nShell commands have full internet access.\n\nUSER REQUEST\nMake a launch trailer'
    ]);
  });

  it('gives shell commands full network only for the Project access choice, fresh and resumed', () => {
    const common = {
      schemaPath: '/workspace/schema.json', outputPath: '/workspace/result.json', prompt: 'Find useful footage',
      imagePaths: [], model: null, reasoningEffort: null, extensionsDir: '/user-data/extensions',
      instructions: 'AGENT INSTRUCTIONS'
    };
    const configs = (argv: string[]) => argv.flatMap((arg, index) => argv[index - 1] === '--config' ? [arg] : []);
    for (const sessionId of [null, 'thread-123']) {
      const argv = buildAutonomousArgv({ ...common, access: 'project', shellNetwork: true, sessionId });
      // exec options, so they apply to `exec resume` too, and never a legacy sandbox mode.
      const network = argv.indexOf('permissions.powermove.network.enabled=true');
      expect(network).toBeGreaterThan(argv.indexOf(sessionId ? 'resume' : 'exec'));
      expect(argv[network - 1]).toBe('--config');
      expect(configs(argv)).toEqual(expect.arrayContaining([
        `default_permissions="${PROJECT_PERMISSION_PROFILE}"`, 'permissions.powermove.extends=":workspace"'
      ]));
      // No proxy and no domain list, so every host is reachable.
      expect(argv.join(' ')).not.toMatch(/network_proxy|network\.domains/);
      expect(argv.at(-1)).toContain('SHELL NETWORK\nShell commands have full internet access.');
      expect(argv).toContain('--search');
      expect(argv).not.toContain('--sandbox');
      expect(argv).not.toContain('--approve-for-me');
      expect(argv).not.toContain('--dangerously-bypass-approvals-and-sandbox');
      expect(argv.join(' ')).not.toMatch(/writable_roots|danger-full-access|sandbox_mode|network_access/);
    }
    // Edit project collapses to project authority but keeps the shell offline.
    const offline = buildAutonomousArgv({ ...common, access: 'project', sessionId: null });
    expect(configs(offline)).toEqual(expect.arrayContaining([`default_permissions="${PROJECT_PERMISSION_PROFILE}"`]));
    expect(offline.join(' ')).not.toMatch(/network_proxy|network\.enabled|SHELL NETWORK/);
    const computer = buildAutonomousArgv({ ...common, access: 'computer', shellNetwork: true, sessionId: null });
    expect(computer.join(' ')).not.toMatch(/network_proxy|default_permissions/);
    expect(buildEditorArgv(common).join(' ')).not.toMatch(/network_proxy|default_permissions/);
    expect(buildEditorArgv(common)).toEqual(expect.arrayContaining(['--sandbox', 'read-only']));
  });

  it('leaves workspace trust to Codex, so project config loads normally', () => {
    for (const sessionId of [null, 'thread-123']) {
      const argv = buildAutonomousArgv({
        schemaPath: '/workspace/schema.json', outputPath: '/workspace/result.json', prompt: 'Build',
        imagePaths: [], model: null, reasoningEffort: null, extensionsDir: '/user-data/extensions',
        instructions: 'AGENT INSTRUCTIONS', access: 'project', sessionId
      });
      expect(argv.join(' ')).not.toMatch(/projects=|trust_level/);
    }
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
      })
    ).toEqual([
      '--search',
      '--dangerously-bypass-approvals-and-sandbox',
      '--add-dir',
      '/user-data/extensions',
      'exec',
      'resume',
      '--skip-git-repo-check',
      '--output-schema',
      '/workspace/.powermove/result-schema.json',
      '--output-last-message',
      '/workspace/.powermove/result-run-2.json',
      '--json',
      '--model',
      'gpt-5-codex',
      '--config',
      'model_reasoning_effort="medium"',
      '--',
      'thread-123',
      'AGENT INSTRUCTIONS\n\nUSER REQUEST\nPublish the approved deliverable'
    ]);
  });

  it('lets a sandboxed run ask the automatic reviewer for an unsandboxed command, and keeps Powermove tools approved', () => {
    const common = {
      schemaPath: '/workspace/schema.json', outputPath: '/workspace/result.json', prompt: 'Build',
      imagePaths: [], model: null, reasoningEffort: null, extensionsDir: '/user-data/extensions', instructions: 'AGENT INSTRUCTIONS',
      nativeTools: { command: '/Applications/Powermove.app/Contents/MacOS/Powermove', args: ['mcp-server.mjs'], env: {} }
    };
    for (const sessionId of [null, 'thread-123']) {
      const argv = buildAutonomousArgv({ ...common, access: 'project', shellNetwork: true, sessionId });
      const reviewer = argv.indexOf('approvals_reviewer="auto_review"');
      // exec options, so `exec resume` gets them too.
      expect(reviewer).toBeGreaterThan(argv.indexOf(sessionId ? 'resume' : 'exec'));
      expect(argv[argv.indexOf('approval_policy="on-request"') - 1]).toBe('--config');
      expect(argv).not.toContain('approval_policy="never"');
      expect(argv).not.toContain('--approve-for-me');
      expect(argv).toContain('mcp_servers.powermove.default_tools_approval_mode="approve"');
    }
  });

  it('turns Codex approvals off in a supervised run, which asks through run_outside_sandbox instead', () => {
    const common = {
      schemaPath: '/workspace/schema.json', outputPath: '/workspace/result.json', prompt: 'Install DM Sans',
      imagePaths: [], model: null, reasoningEffort: null, extensionsDir: '/user-data/extensions', instructions: 'AGENT INSTRUCTIONS',
      nativeTools: { command: '/Applications/Powermove.app/Contents/MacOS/Powermove', args: ['mcp-server.mjs'], env: {}, toolTimeoutSec: 4200 },
      sessionId: null
    };
    const argv = buildAutonomousArgv({ ...common, access: 'project', shellNetwork: true, approval: 'supervised' });
    expect(argv).toContain('approval_policy="never"');
    expect(argv).not.toContain('approvals_reviewer="auto_review"');
    expect(argv).toContain('default_permissions="powermove"');
    expect(argv).toContain('mcp_servers.powermove.tool_timeout_sec=4200');
    expect(argv.at(-1)).toContain('run_outside_sandbox');

    const auto = buildAutonomousArgv({ ...common, access: 'project', shellNetwork: true, approval: 'auto' });
    expect(auto).toContain('approvals_reviewer="auto_review"');
    expect(auto.at(-1)).not.toContain('run_outside_sandbox');
  });

  it('passes the session id after -- and refuses one that is not an id', () => {
    const common = {
      schemaPath: '/workspace/schema.json', outputPath: '/workspace/result.json', prompt: '--help',
      imagePaths: ['/workspace/a.png'], model: null, reasoningEffort: null, extensionsDir: '/user-data/extensions',
      instructions: 'AGENT INSTRUCTIONS', access: 'project' as const
    };
    const argv = buildAutonomousArgv({ ...common, sessionId: '019999aa-0000-7000-8000-000000000000' });
    const separator = argv.indexOf('--');
    expect(argv.slice(separator + 1, separator + 2)).toEqual(['019999aa-0000-7000-8000-000000000000']);
    expect(argv.indexOf('--image')).toBeLessThan(separator);
    expect(argv).toHaveLength(separator + 3);
    for (const sessionId of ['--dangerously-bypass-approvals-and-sandbox', '-c', 'a b', '../thread']) {
      expect(() => buildAutonomousArgv({ ...common, sessionId }), sessionId).toThrow('Invalid Codex session id.');
    }
  });

  it('allows user resources instead of suppressing integrations, rules and skills', () => {
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
    });
    expect(argv).not.toContain('--ignore-user-config');
    expect(argv).not.toContain('--ignore-rules');
    expect(argv).not.toContain('--disable');
    expect(argv).not.toContain('mcp_servers={}');
    expect(argv.join(' ')).not.toContain('skills.');
  });

  it('adds user tools and the run-scoped Powermove MCP server without clearing inherited resources', () => {
    const argv = buildAutonomousArgv({
      schemaPath: '/workspace/schema.json',
      outputPath: '/workspace/result.json',
      prompt: 'Inspect and edit the live composition',
      imagePaths: [],
      model: null,
      reasoningEffort: null,
      access: 'project',
      extensionsDir: '/workspace/extensions',
      sessionId: null,
      instructions: 'AGENT INSTRUCTIONS',
      externalMcpServers: { external: { command: 'fixture-server' }, powermove: { command: 'wrong-server' } },
      nativeTools: {
        command: '/Applications/Powermove.app/Contents/MacOS/Powermove',
        args: ['/Applications/Powermove.app/Contents/Resources/agent-tools/mcp-server.mjs'],
        env: { POWERMOVE_AGENT_TOOL_TOKEN: 'secret', ELECTRON_RUN_AS_NODE: '1' }
      }
    });
    const command = argv.indexOf('mcp_servers.powermove.command="/Applications/Powermove.app/Contents/MacOS/Powermove"');
    expect(command).toBeGreaterThan(-1);
    expect(argv.indexOf('mcp_servers."external"={"command"="fixture-server"}')).toBeGreaterThan(-1);
    expect(argv.join(' ')).not.toContain('wrong-server');
    expect(argv).toContain('mcp_servers.powermove.args=["/Applications/Powermove.app/Contents/Resources/agent-tools/mcp-server.mjs"]');
    expect(argv).toContain('mcp_servers.powermove.env.ELECTRON_RUN_AS_NODE="1"');
    expect(argv).toContain('mcp_servers.powermove.env.POWERMOVE_AGENT_TOOL_TOKEN="secret"');
    expect(argv).toContain('mcp_servers.powermove.required=true');
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

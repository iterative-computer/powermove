import { execFile } from 'node:child_process';
import type { CodexAccess, ReasoningEffort } from '../../shared/ipc';
import { discoverCodexBinary } from './env';

export const ADAPTER_VERSION = '1';

export const REQUIRED_CODEX_FLAGS = [
  '--ephemeral',
  '--skip-git-repo-check',
  '--ignore-rules',
  '--sandbox',
  '--output-schema',
  '--output-last-message',
  '--json',
  '--model',
  '--config',
  '--image',
  '--search',
  '--approve-for-me',
  '--dangerously-bypass-approvals-and-sandbox'
] as const;

interface CommonArgvOptions {
  schemaPath: string;
  outputPath: string;
  prompt: string;
  imagePaths: readonly string[];
  model: string | null;
  reasoningEffort: ReasoningEffort | null;
}

export interface EditorArgvOptions extends CommonArgvOptions {}

export interface AutonomousArgvOptions extends CommonArgvOptions {
  access: Exclude<CodexAccess, 'editor'>;
  sessionId: string | null;
  instructions: string;
}

function appendModelOptions(
  argv: string[],
  model: string | null,
  reasoningEffort: ReasoningEffort | null
): void {
  if (model !== null) argv.push('--model', model);
  if (reasoningEffort !== null) {
    argv.push('--config', `model_reasoning_effort="${reasoningEffort}"`);
  }
}

function appendPromptAndImages(argv: string[], prompt: string, imagePaths: readonly string[]): void {
  argv.push(prompt);
  for (const imagePath of imagePaths) argv.push('--image', imagePath);
}

export function buildEditorArgv(options: EditorArgvOptions): string[] {
  const argv = [
    'exec',
    '--ephemeral',
    '--skip-git-repo-check',
    '--ignore-rules',
    '--sandbox',
    'read-only',
    '--output-schema',
    options.schemaPath,
    '--output-last-message',
    options.outputPath,
    '--json'
  ];
  appendModelOptions(argv, options.model, options.reasoningEffort);
  appendPromptAndImages(argv, options.prompt, options.imagePaths);
  return argv;
}

export function buildAutonomousArgv(options: AutonomousArgvOptions): string[] {
  const argv = ['--search'];
  if (options.access === 'computer') {
    argv.push('--dangerously-bypass-approvals-and-sandbox');
  } else {
    // codex ≥ 0.147 rejects an explicit --sandbox alongside --approve-for-me;
    // --approve-for-me itself routes approvals through the workspace-write
    // sandbox (the Swift shell's flag pair predates that change).
    argv.push('--approve-for-me');
  }

  argv.push('exec');
  if (options.sessionId !== null && options.sessionId.trim() !== '') argv.push('resume');
  argv.push(
    '--skip-git-repo-check',
    '--output-schema',
    options.schemaPath,
    '--output-last-message',
    options.outputPath,
    '--json'
  );
  appendModelOptions(argv, options.model, options.reasoningEffort);
  if (options.sessionId !== null && options.sessionId.trim() !== '') argv.push(options.sessionId.trim());
  const fullPrompt = `${options.instructions}\n\nUSER REQUEST\n${options.prompt}`;
  appendPromptAndImages(argv, fullPrompt, options.imagePaths);
  return argv;
}

export interface AdapterCapabilities {
  adapterVersion: string;
  supported: string[];
  missing: string[];
}

function execFileText(file: string, args: readonly string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(file, [...args], { encoding: 'utf8', timeout: 5_000, maxBuffer: 2 * 1024 * 1024 }, (error, stdout) => {
      if (error) reject(error);
      else resolve(stdout);
    });
  });
}

export async function capabilities(binary?: string | null): Promise<AdapterCapabilities> {
  const executable = binary ?? await discoverCodexBinary(null);
  const help = await execFileText(executable, ['exec', '--help']);
  const supported = REQUIRED_CODEX_FLAGS.filter((flag) => help.includes(flag));
  const missing = REQUIRED_CODEX_FLAGS.filter((flag) => !help.includes(flag));
  return { adapterVersion: ADAPTER_VERSION, supported: [...supported], missing: [...missing] };
}

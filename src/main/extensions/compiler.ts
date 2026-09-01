import { createHash, randomUUID } from 'node:crypto';
import { mkdir, readFile, realpath, rename, rm, stat, writeFile, lstat } from 'node:fs/promises';
import path from 'node:path';

import type { Loader, Plugin } from 'esbuild';
import { EXTENSION_ID, isSafeEntry, MANIFEST_LIMITS } from '../../shared/extensions';
import { loadEsbuild } from './esbuild-binary';

export type CompileExtensionResult =
  | { ok: true; bundlePath: string; hash: string }
  | { ok: false; error: string };

interface CompileExtensionOptions {
  dir: string;
  entry: string;
  outDir: string;
}

interface SvelteCompiler {
  compile(source: string, options: Record<string, unknown>): { js: { code: string } };
  compileModule(source: string, options: Record<string, unknown>): { js: { code: string } };
}

const SOURCE_SUFFIXES = ['.ts', '.js', '.mjs', '.svelte', '.svelte.ts', '.svelte.js'] as const;
const RUNTIME_NAMESPACE = 'powermove-runtime';
const SOURCE_NAMESPACE = 'powermove-source';
const RESERVED_BINDINGS = new Set([
  'await', 'break', 'case', 'catch', 'class', 'const', 'continue', 'debugger', 'default',
  'delete', 'do', 'else', 'enum', 'export', 'extends', 'false', 'finally', 'for', 'function',
  'if', 'implements', 'import', 'in', 'instanceof', 'interface', 'let', 'new', 'null',
  'package', 'private', 'protected', 'public', 'return', 'static', 'super', 'switch', 'this',
  'throw', 'true', 'try', 'typeof', 'var', 'void', 'while', 'with', 'yield'
]);

let svelteCompilerPromise: Promise<SvelteCompiler> | undefined;
const runtimeExports = new Map<string, Promise<string[]>>();

function getSvelteCompiler(): Promise<SvelteCompiler> {
  svelteCompilerPromise ??= import('svelte/compiler') as Promise<SvelteCompiler>;
  return svelteCompilerPromise;
}

function truncateError(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error);
  return text.slice(0, MANIFEST_LIMITS.errorChars);
}

function isContained(root: string, candidate: string): boolean {
  const relative = path.relative(root, candidate);
  return relative === '' || (!relative.startsWith(`..${path.sep}`) && relative !== '..' && !path.isAbsolute(relative));
}

function hasNodeModulesComponent(file: string): boolean {
  return file.split(path.sep).includes('node_modules');
}

function isRuntimeImport(specifier: string): boolean {
  return specifier === 'powermove' || specifier === 'svelte' || specifier.startsWith('svelte/');
}

function sourceLoader(file: string): Loader {
  if (file.endsWith('.svelte.ts')) return 'ts';
  if (file.endsWith('.svelte.js')) return 'js';
  if (file.endsWith('.ts')) return 'ts';
  return 'js';
}

async function namespaceKeys(specifier: string): Promise<string[]> {
  let pending = runtimeExports.get(specifier);
  if (!pending) {
    pending = importRuntimeNamespace(specifier).then((module) => Object.keys(module));
    runtimeExports.set(specifier, pending);
  }
  return pending;
}

async function importRuntimeNamespace(specifier: string): Promise<Record<string, unknown>> {
  if (specifier === 'svelte') return import('svelte');
  if (specifier === 'svelte/store') return import('svelte/store');
  if (specifier === 'svelte/internal/client') {
    // @ts-expect-error -- Svelte intentionally ships no declarations for its internal client entrypoint.
    return import('svelte/internal/client');
  }
  throw new Error(`Unsupported Svelte runtime import: ${specifier}`);
}

async function runtimeModule(specifier: string): Promise<string> {
  if (specifier === 'powermove' || specifier === 'svelte/internal/disclose-version') return 'export {}';

  const keys = await namespaceKeys(specifier);
  const declarations = keys
    .filter((key) => key !== 'default' && /^[A-Za-z_$][\w$]*$/.test(key) && !RESERVED_BINDINGS.has(key))
    .map((key) => `export const ${key} = m.${key};`);
  const reservedExports = keys
    .filter((key) => RESERVED_BINDINGS.has(key) && key !== 'default')
    .map((key, index) => `const __reserved_${index} = m.${key}; export { __reserved_${index} as ${key} };`);
  return [
    `const m = globalThis.__powermove_runtime[${JSON.stringify(specifier)}];`,
    'export default m;',
    ...declarations,
    ...reservedExports
  ].join('\n');
}

/** Compile a user or project extension into one browser ESM bundle. */
export async function compileExtension({ dir, entry, outDir }: CompileExtensionOptions): Promise<CompileExtensionResult> {
  try {
    const esbuild = await loadEsbuild();
    const id = path.basename(path.resolve(dir));
    if (!EXTENSION_ID.test(id)) throw new Error(`Invalid extension id: ${id}`);
    if (!isSafeEntry(entry)) throw new Error('Entry must be a relative .ts, .js, or .mjs source path');

    const extensionRoot = await realpath(dir);
    const seenSources = new Set<string>();
    let sourceBytes = 0;

    const resolveSource = async (specifier: string, resolveDir: string): Promise<string> => {
      if (path.isAbsolute(specifier)) throw new Error(`Absolute imports are not allowed: ${specifier}`);
      if (!specifier.startsWith('./') && !specifier.startsWith('../')) {
        throw new Error(`Unsupported bare import: ${specifier}`);
      }

      const unresolved = path.resolve(resolveDir, specifier);
      if (!isContained(extensionRoot, unresolved) || hasNodeModulesComponent(path.relative(extensionRoot, unresolved))) {
        throw new Error(`Import escapes extension directory: ${specifier}`);
      }

      const candidates = path.extname(unresolved)
        ? [unresolved]
        : [unresolved, ...SOURCE_SUFFIXES.map((suffix) => `${unresolved}${suffix}`), ...SOURCE_SUFFIXES.map((suffix) => path.join(unresolved, `index${suffix}`))];

      for (const candidate of candidates) {
        try {
          if (!(await stat(candidate)).isFile()) continue;
          const canonical = await realpath(candidate);
          if (!isContained(extensionRoot, canonical) || hasNodeModulesComponent(path.relative(extensionRoot, canonical))) {
            throw new Error(`Import escapes extension directory: ${specifier}`);
          }
          return canonical;
        } catch (error) {
          if (error instanceof Error && error.message.startsWith('Import escapes')) throw error;
        }
      }
      throw new Error(`Could not resolve source import: ${specifier}`);
    };

    const plugin: Plugin = {
      name: 'powermove-extension-boundary',
      setup(build) {
        build.onResolve({ filter: /.*/ }, async (args) => {
          if (isRuntimeImport(args.path)) return { path: args.path, namespace: RUNTIME_NAMESPACE };
          const resolveDir = args.kind === 'entry-point' ? extensionRoot : args.resolveDir;
          const specifier = args.kind === 'entry-point' && !args.path.startsWith('.') ? `./${args.path}` : args.path;
          return { path: await resolveSource(specifier, resolveDir), namespace: SOURCE_NAMESPACE };
        });

        build.onLoad({ filter: /.*/, namespace: RUNTIME_NAMESPACE }, async (args) => ({
          contents: await runtimeModule(args.path),
          loader: 'js'
        }));

        build.onLoad({ filter: /.*/, namespace: SOURCE_NAMESPACE }, async (args) => {
          if (!seenSources.has(args.path)) {
            const info = await lstat(args.path);
            if (info.size > MANIFEST_LIMITS.sourceBytes) {
              throw new Error(`Extension exceeds source size limit (${MANIFEST_LIMITS.sourceBytes} bytes)`);
            }
            seenSources.add(args.path);
            sourceBytes += info.size;
            if (seenSources.size > MANIFEST_LIMITS.sourceFiles) {
              throw new Error(`Extension exceeds source file limit (${MANIFEST_LIMITS.sourceFiles})`);
            }
            if (sourceBytes > MANIFEST_LIMITS.sourceBytes) {
              throw new Error(`Extension exceeds source size limit (${MANIFEST_LIMITS.sourceBytes} bytes)`);
            }
          }

          const source = (await readFile(args.path)).toString('utf8');
          if (args.path.endsWith('.svelte')) {
            const compiler = await getSvelteCompiler();
            const compiled = compiler.compile(source, {
              generate: 'client',
              css: 'injected',
              filename: args.path,
              runes: true
            });
            return { contents: compiled.js.code, loader: 'js', resolveDir: path.dirname(args.path) };
          }
          if (args.path.endsWith('.svelte.ts') || args.path.endsWith('.svelte.js')) {
            const compiler = await getSvelteCompiler();
            const compiled = compiler.compileModule(source, {
              generate: 'client',
              filename: args.path,
              runes: true
            });
            return { contents: compiled.js.code, loader: 'js', resolveDir: path.dirname(args.path) };
          }
          return { contents: source, loader: sourceLoader(args.path), resolveDir: path.dirname(args.path) };
        });
      }
    };

    const result = await esbuild.build({
      entryPoints: [entry],
      bundle: true,
      format: 'esm',
      platform: 'browser',
      target: 'es2022',
      write: false,
      sourcemap: 'inline',
      logLevel: 'silent',
      absWorkingDir: dir,
      plugins: [plugin]
    });
    const output = result.outputFiles[0]?.contents;
    if (!output) throw new Error('Compiler produced no JavaScript output');
    if (output.byteLength > MANIFEST_LIMITS.bundleBytes) {
      throw new Error(`Extension exceeds bundle size limit (${MANIFEST_LIMITS.bundleBytes} bytes)`);
    }

    const targetDirectory = path.join(outDir, id);
    const bundlePath = path.join(targetDirectory, 'bundle.js');
    const temporaryPath = path.join(targetDirectory, `.bundle-${randomUUID()}.tmp`);
    await mkdir(targetDirectory, { recursive: true });
    try {
      await writeFile(temporaryPath, output, { flag: 'wx' });
      await rename(temporaryPath, bundlePath);
    } finally {
      await rm(temporaryPath, { force: true });
    }

    return {
      ok: true,
      bundlePath,
      hash: createHash('sha256').update(output).digest('hex').slice(0, 16)
    };
  } catch (error) {
    return { ok: false, error: truncateError(error) };
  }
}

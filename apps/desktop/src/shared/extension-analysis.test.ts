import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { analyzeExtensionSource, DEPRECATED_MEMBERS, formatDeprecationWarning, type DeprecationWarning } from './extension-analysis';

const run = (text: string, file = 'index.ts') => analyzeExtensionSource([{ path: file, text }]);
const brief = (warnings: DeprecationWarning[]) => warnings.map((w) => `${w.line}:${w.member}:${w.access}`);

describe('analyzeExtensionSource', () => {
  it('finds direct member chains with file, line, column and replacement', () => {
    const warnings = run(`export default function activate(api) {\n  const url = api.render.snapshot(0, 480);\n  api.render.renderFrameTo(0, 10, 10);\n  api.render.raster(layer);\n  const gl = api.render.gl.context;\n}`);
    expect(brief(warnings)).toEqual(['2:render.snapshot:static', '3:render.renderFrameTo:static', '4:render.raster:static', '5:render.gl.context:static']);
    expect(warnings[0]).toMatchObject({ file: 'index.ts', column: 26, replacement: 'await api.render.snapshotAsync(time, maxWidth)' });
    expect(formatDeprecationWarning(warnings[0]!)).toMatch(/^index\.ts:2: api\.render\.snapshot is a deprecated compatibility API\. It keeps working at apiVersion 1 to 3; new code should use await api\.render\.snapshotAsync/);
    expect(warnings[0]!.message).toContain('change it only if they agree');
  });

  it('follows the activate parameter under any name and PowermoveAPI annotations', () => {
    expect(brief(run(`export default (ext) => { ext.render.snapshot(1); }`))).toEqual(['1:render.snapshot:static']);
    expect(brief(run(`export default async function (p) {\n p.render.gl.context }`))).toEqual(['2:render.gl.context:static']);
    expect(brief(run(`function helper(host: PowermoveAPI) {\n return host.render.raster(l) }`))).toEqual(['2:render.raster:static']);
  });

  it('follows aliases, destructuring and computed literal keys', () => {
    const text = [
      'export default function activate(api) {',
      '  const render = api.render;',
      '  render.snapshot(0);',
      '  const { renderFrameTo, gl: { context } } = api.render;',
      "  api.render['raster'](layer);",
      '  const r2 = render;',
      '  r2?.gl?.context;',
      '  const { render: { snapshot: shot } } = api;',
      '}'
    ].join('\n');
    expect(brief(run(text))).toEqual([
      '3:render.snapshot:alias',
      '4:render.renderFrameTo:destructured',
      '4:render.gl.context:destructured',
      '5:render.raster:computed',
      '7:render.gl.context:alias',
      '8:render.snapshot:destructured'
    ]);
  });

  it('reports a computed key that is not a literal as a possible use', () => {
    const warnings = run('const name = pick();\napi.render[name](0);');
    expect(brief(warnings)).toEqual(['2:render.raster:dynamic', '2:render.renderFrameTo:dynamic', '2:render.snapshot:dynamic']);
    expect(warnings[0]!.message).toMatch(/^This computed access may reach api\.render\.raster/);
  });

  it('finds the raster service override, including type arguments and a constant name', () => {
    const text = [
      "const original = api.services.get<Raster>('raster');",
      'const services = api.services;',
      "services.register('raster', wrap);",
      "const NAME = 'raster';",
      'api.services.get(NAME);',
      "api.services.get('viewer');"
    ].join('\n');
    expect(brief(run(text))).toEqual(['1:services.raster:static', '3:services.raster:static', '5:services.raster:alias']);
  });

  it('finds live media fields on assets.get results', () => {
    const text = [
      'const live = api.media.assets.get(id);',
      'video.src = live.url;',
      'live?.el?.play();',
      "if (live.audioBlob instanceof Blob) {}",
      'api.media.assets.get(id)!.object3d.traverse(f);',
      'const { peaks } = live;'
    ].join('\n');
    expect(brief(run(text))).toEqual([
      '2:media.assets.get().url:alias',
      '3:media.assets.get().el:alias',
      '4:media.assets.get().audioBlob:alias',
      '5:media.assets.get().object3d:static'
    ]);
  });

  it('finds host.pm and host.state but not the live host members', () => {
    expect(brief(run('api.host.pm.proj;\nconst { state, mount } = api.host;\napi.host.mount(C, el, {});\napi.host.haptic.alignment();')))
      .toEqual(['1:host.pm:static', '2:host.state:destructured']);
  });

  it('ignores comments, strings, regular expressions and unrelated objects', () => {
    const text = [
      '// api.render.snapshot(0)',
      '/* api.render.raster(l) */',
      "const s = 'api.render.snapshot';",
      'const re = /api.render.snapshot/g;',
      'other.render.snapshot(0);',
      'const t = `api.render.snapshot ${1 + 2}`;',
      'api.render.snapshotAsync(0);',
      'api.render.rasterAsync(l);',
      'api.render.gl.bounds(l, 0);'
    ].join('\n');
    expect(run(text)).toEqual([]);
  });

  it('reads template expressions and Svelte scripts and markup expressions with their real lines', () => {
    expect(brief(run('const t = `${api.render.snapshot(0)}`;\n'))).toEqual(['1:render.snapshot:static']);
    const svelte = [
      '<script lang="ts">',
      '  let { api }: { api: PowermoveAPI } = $props();',
      "  const shot = () => api.render.snapshot(0);",
      '</script>',
      "<p>Don't call api.render.raster here: it is text</p>",
      '{#if api.render.gl.context}',
      '  <button onclick={() => api.render.renderFrameTo(0, 1, 1)}>Go</button>',
      '{/if}'
    ].join('\n');
    expect(brief(run(svelte, 'Panel.svelte'))).toEqual(['3:render.snapshot:static', '6:render.gl.context:static', '7:render.renderFrameTo:static']);
  });

  it('skips tests, declarations, non-source files and generated bundles', () => {
    const text = 'api.render.snapshot(0);';
    expect(analyzeExtensionSource([
      { path: 'index.test.ts', text },
      { path: 'types.d.ts', text },
      { path: 'README.md', text },
      { path: 'node_modules/x/index.js', text },
      { path: 'big.js', text: text + ' '.repeat(2 * 1024 * 1024) }
    ])).toEqual([]);
  });

  it('never throws on malformed source', () => {
    for (const text of ['api.render[', 'const {', '`${', '/* never closed', "'unterminated", 'api.services.get<', '<script>api.render.snapshot(']) {
      expect(() => run(text)).not.toThrow();
      expect(() => run(text, 'X.svelte')).not.toThrow();
    }
  });

  it('reports each member once per line and sorts by file and line', () => {
    const warnings = analyzeExtensionSource([
      { path: 'b.ts', text: 'api.render.snapshot(0); api.render.snapshot(1);' },
      { path: 'a.ts', text: '\napi.render.raster(l);' }
    ]);
    expect(warnings.map((w) => `${w.file}:${w.line}:${w.member}`)).toEqual(['a.ts:2:render.raster', 'b.ts:1:render.snapshot']);
  });

  it('finds every deprecated use in the shipped typewriter fixture', () => {
    const dir = path.resolve(__dirname, '../../e2e/fixtures/typewriter-effect');
    const files = ['index.ts', 'raster.ts', 'cursor.ts', 'typing.js'].map((name) => ({ path: name, text: readFileSync(path.join(dir, name), 'utf8') }));
    expect(brief(analyzeExtensionSource(files)).filter((line) => line.includes('services.raster'))).toEqual(['34:services.raster:static', '107:services.raster:static']);
  });

  it('keeps EXTENSIONS.md table equal to the member list', () => {
    const doc = readFileSync(path.resolve(__dirname, '../../docs/EXTENSIONS.md'), 'utf8');
    for (const member of DEPRECATED_MEMBERS) {
      expect(doc, member.id).toContain(`| \`${member.id}\` | \`${member.replacement}\``);
    }
  });
});

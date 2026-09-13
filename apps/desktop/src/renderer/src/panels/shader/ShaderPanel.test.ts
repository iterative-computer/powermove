// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { frameBus } from '../../runtime/frame-bus';
import { doc } from '../../state/document.svelte';
import { setSelection } from '../../state/selection.svelte';
import ShaderPanel from '../ShaderPanel.svelte';

const SHADER_CODE = 'void main() { fragColor = vec4(1.0); }';
const PRESET_CODE = 'void main() { fragColor = vec4(0.0); }';

let target: HTMLDivElement;
let instance: Record<string, any> | undefined;

function shaderLayer(id = 'shader-1', code = SHADER_CODE): Record<string, any> {
  return {
    id,
    type: 'shader',
    name: 'Shader',
    d: { code, w: 1920, h: 1080, uniforms: {} },
    p: {},
    fx: [],
    masks: []
  };
}

function setup(selectedLayer: Record<string, any>, layers = [selectedLayer]) {
  const project = { id: 'project-1', layers, assets: {} } as any;
  const meta = { shaderKey: 'sh:shader-1:hash', udefs: [{ name: 'uSpeed' }] };
  const writeCode = (command: Record<string, any>) => {
    const targetLayer = project.layers.find((candidate: any) => candidate.id === command.target);
    if (targetLayer && command.type === 'set_content' && command.patch?.code !== undefined) {
      targetLayer.d.code = command.patch.code;
      doc.bump('values');
    }
  };
  const PM = {
    proj: project,
    SHADER_PRESETS: { 'Aurora Field': PRESET_CODE },
    Edit: {
      begin: vi.fn(),
      dispatch: vi.fn(writeCode),
      commit: vi.fn(),
      cancel: vi.fn(),
      apply: vi.fn((command: Record<string, any>) => {
        writeCode(command);
        return { ok: true };
      })
    },
    UIState: { getShaderMeta: vi.fn(() => meta) },
    GL: {
      compileError: vi.fn(() => null),
      dropProgram: vi.fn()
    },
    invalidate: vi.fn(),
    Inspector: { refresh: vi.fn() }
  } as Record<string, any>;
  PM.Kernel = {
    api: () => ({
      edit: PM.Edit,
      history: { begin: vi.fn(), commit: vi.fn(), cancel: vi.fn(), do: vi.fn((_label: string, operation: () => unknown) => operation()) }
    })
  };

  window.PM = PM as any;
  doc.replace(project);
  setSelection({ layers: [selectedLayer.id], keys: [], chan: null });
  instance = mount(ShaderPanel, { target, props: { panelId: 'shader', spec: {} } });
  flushSync();
  return { PM, project, meta };
}

beforeEach(() => {
  target = document.createElement('div');
  document.body.append(target);
});

afterEach(async () => {
  if (instance) await unmount(instance);
  instance = undefined;
  frameBus.clear();
  setSelection({ layers: [], keys: [], chan: null });
  target.remove();
  delete window.PM;
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('ShaderPanel', () => {
  it('shows the selected shader layer source in a real labelled textarea', () => {
    setup(shaderLayer());

    const textarea = target.querySelector<HTMLTextAreaElement>('textarea[data-shader-editor]');
    expect(textarea?.value).toBe(SHADER_CODE);
    expect(textarea?.getAttribute('spellcheck')).toBe('false');
    expect(textarea?.getAttribute('aria-label')).toBe('Shader source code');
    expect(target.textContent).toContain('Escape, then Tab');
    expect(target.querySelector('label[for="shader-preset-shader"]')?.textContent).toBe('Shader preset');
  });

  it('uses begin/write/commit for debounced typing', async () => {
    vi.useFakeTimers();
    const { PM } = setup(shaderLayer());
    const textarea = target.querySelector<HTMLTextAreaElement>('textarea[data-shader-editor]')!;

    textarea.dispatchEvent(new FocusEvent('focus'));
    textarea.value = `${SHADER_CODE}\n// changed`;
    flushSync(() => textarea.dispatchEvent(new InputEvent('input', { bubbles: true })));

    expect(PM.Edit.begin).toHaveBeenCalledWith('Edit shader', { origin: 'shader-panel' });
    await vi.advanceTimersByTimeAsync(419);
    expect(PM.Edit.dispatch).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(PM.Edit.dispatch).toHaveBeenCalledWith({
      type: 'set_content',
      target: 'shader-1',
      patch: { code: `${SHADER_CODE}\n// changed` }
    });
    expect(PM.Edit.commit).toHaveBeenCalledWith('Edit shader');
    expect(PM.Edit.begin).toHaveBeenCalledTimes(2);
    expect(textarea.value).toBe(`${SHADER_CODE}\n// changed`);

    textarea.dispatchEvent(new FocusEvent('blur'));
    expect(PM.Edit.commit).toHaveBeenCalledTimes(2);
  });

  it('applies a preset as a one-shot set_content command', () => {
    const { PM } = setup(shaderLayer());
    const select = target.querySelector<HTMLSelectElement>('#shader-preset-shader')!;

    select.value = 'Aurora Field';
    select.dispatchEvent(new Event('change', { bubbles: true }));

    expect(PM.Edit.apply).toHaveBeenCalledWith(
      { type: 'set_content', target: 'shader-1', patch: { code: PRESET_CODE } },
      { label: 'Shader preset', origin: 'shader-panel' }
    );
    expect(select.value).toBe('');
    flushSync();
    expect(target.querySelector<HTMLTextAreaElement>('textarea')?.value).toBe(PRESET_CODE);
    expect(PM.invalidate).toHaveBeenCalledWith();
  });

  it('surfaces the compositor compile error in a polite live region', () => {
    const { PM, meta } = setup(shaderLayer());
    const firstLine = `ERROR: 0:7: ${'unexpected token '.repeat(8)}`;
    PM.GL.compileError.mockReturnValue(`${firstLine}\nsecond line`);
    doc.bump('values');
    flushSync();

    const status = target.querySelector<HTMLElement>('[role="status"]');
    expect(PM.GL.compileError).toHaveBeenCalledWith(meta.shaderKey);
    expect(status?.textContent).toBe(firstLine.slice(0, 90));
    expect(status?.title).toBe(firstLine.slice(0, 90));
    expect(status?.getAttribute('aria-live')).toBe('polite');
    expect(status?.classList.contains('bad')).toBe(true);
  });

  it('falls back from a non-shader selection to the first project shader', () => {
    const textLayer = { id: 'text-1', type: 'text', name: 'Title', d: { text: 'Hello' } };
    const firstShader = shaderLayer('shader-first', '// first');
    const secondShader = shaderLayer('shader-second', '// second');
    setup(textLayer, [textLayer, firstShader, secondShader]);

    expect(target.querySelector<HTMLTextAreaElement>('textarea')?.value).toBe('// first');
    expect(target.textContent).not.toContain('No shader layers');
  });

  it('shows the empty state only when the project has no shader layers', () => {
    const textLayer = { id: 'text-1', type: 'text', name: 'Title', d: { text: 'Hello' } };
    setup(textLayer);

    expect(target.querySelector('textarea')).toBeNull();
    expect(target.textContent).toContain('No shader layers');
    expect(target.textContent).toContain('Create a shader layer');
  });

  it('inserts two spaces with Tab and lets Escape then Tab leave the field', async () => {
    vi.useFakeTimers();
    const { PM } = setup(shaderLayer('shader-1', 'ab'));
    const textarea = target.querySelector<HTMLTextAreaElement>('textarea')!;
    textarea.dispatchEvent(new FocusEvent('focus'));
    textarea.setSelectionRange(1, 1);

    const insertTab = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    textarea.dispatchEvent(insertTab);
    expect(insertTab.defaultPrevented).toBe(true);
    expect(textarea.value).toBe('a  b');
    await vi.advanceTimersByTimeAsync(420);
    expect(PM.Edit.dispatch).toHaveBeenCalledWith({
      type: 'set_content', target: 'shader-1', patch: { code: 'a  b' }
    });

    textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    const exitTab = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    textarea.dispatchEvent(exitTab);
    expect(exitTab.defaultPrevented).toBe(false);
    textarea.dispatchEvent(new FocusEvent('blur'));
  });

  it('does not let document ticks clobber a focused draft', () => {
    const candidate = shaderLayer();
    const { project } = setup(candidate);
    const textarea = target.querySelector<HTMLTextAreaElement>('textarea')!;
    textarea.dispatchEvent(new FocusEvent('focus'));
    textarea.value = '// local draft';
    flushSync(() => textarea.dispatchEvent(new InputEvent('input', { bubbles: true })));

    project.layers[0].d.code = '// external replacement';
    flushSync(() => doc.bump('values'));
    expect(textarea.value).toBe('// local draft');

    textarea.dispatchEvent(new FocusEvent('blur'));
    flushSync();
    expect(project.layers[0].d.code).toBe('// local draft');
    expect(textarea.value).toBe('// local draft');
  });

  it('drops the compiled program and invalidates all views', () => {
    const { PM, meta } = setup(shaderLayer());
    const compile = [...target.querySelectorAll<HTMLButtonElement>('button')]
      .find((button) => button.textContent?.includes('Compile'))!;
    compile.click();

    expect(PM.GL.dropProgram).toHaveBeenCalledExactlyOnceWith(meta.shaderKey);
    expect(PM.invalidate).toHaveBeenCalledWith();
    expect(PM.Inspector.refresh).toHaveBeenCalledOnce();
  });

  it('flushes and commits a live edit when unmounted', async () => {
    vi.useFakeTimers();
    const { PM } = setup(shaderLayer());
    const textarea = target.querySelector<HTMLTextAreaElement>('textarea')!;
    textarea.dispatchEvent(new FocusEvent('focus'));
    textarea.value = '// pending at unmount';
    flushSync(() => textarea.dispatchEvent(new InputEvent('input', { bubbles: true })));

    await unmount(instance!);
    instance = undefined;

    expect(PM.Edit.dispatch).toHaveBeenCalledWith({
      type: 'set_content', target: 'shader-1', patch: { code: '// pending at unmount' }
    });
    expect(PM.Edit.commit).toHaveBeenCalledWith('Edit shader');
  });
});

// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import { normalizeExportDefaults } from '../../core/export-defaults';
import {
  createNewProjectForm,
  createProjectSettingsControl,
  type ProjectSettingsBridge
} from './project-settings';

function bridge(overrides: Partial<ProjectSettingsBridge> = {}) {
  const state = { name: 'Reel', w: 1920, h: 1080, fps: 30, dur: 10 };
  let defaults = normalizeExportDefaults({});
  const applyComposition = vi.fn((patch) => Object.assign(state, {
    name: patch.name ?? state.name,
    w: patch.width ?? state.w,
    h: patch.height ?? state.h,
    fps: patch.fps ?? state.fps,
    dur: patch.duration ?? state.dur
  }));
  const applyExportDefaults = vi.fn((patch) => { defaults = normalizeExportDefaults({ ...defaults, ...patch }); });
  return {
    state,
    applyComposition,
    applyExportDefaults,
    api: {
      composition: () => ({ ...state }),
      applyComposition,
      exportDefaults: () => defaults,
      applyExportDefaults,
      ...overrides
    } as ProjectSettingsBridge
  };
}

const field = (control: { element: HTMLElement }, label: string) =>
  control.element.querySelector<HTMLInputElement | HTMLSelectElement>(`[aria-label="${label}"]`)!;

const change = (node: HTMLElement) => node.dispatchEvent(new Event('change'));

describe('project settings control', () => {
  it('shows composition and export sections seeded from the project', () => {
    const host = bridge();
    const control = createProjectSettingsControl(host.api);
    const headings = [...control.element.querySelectorAll('.settings-section-heading > b')]
      .map((node) => node.textContent);
    expect(headings).toEqual(['Composition', 'Export']);
    expect(field(control, 'Project name').value).toBe('Reel');
    expect(field(control, 'Width in pixels').value).toBe('1920');
    expect(field(control, 'Height in pixels').value).toBe('1080');
    expect(field(control, 'Frame rate').value).toBe('30');
    expect(field(control, 'Duration in seconds').value).toBe('10');
    expect(field(control, 'Resolution preset').value).toBe('1920x1080');
    expect(field(control, 'Export format').value).toBe('webm');
    control.destroy();
  });

  it('applies a resolution preset as one composition edit', () => {
    const host = bridge();
    const control = createProjectSettingsControl(host.api);
    const preset = field(control, 'Resolution preset');
    preset.value = '1080x1920';
    change(preset);
    expect(host.applyComposition).toHaveBeenCalledWith({ width: 1080, height: 1920 });
    expect(field(control, 'Width in pixels').value).toBe('1080');
    control.destroy();
  });

  it('clamps out-of-range sizes and durations instead of writing them', () => {
    const host = bridge();
    const control = createProjectSettingsControl(host.api);
    const width = field(control, 'Width in pixels');
    width.value = '999999';
    change(width);
    expect(host.applyComposition).toHaveBeenCalledWith({ width: 8192 });
    const duration = field(control, 'Duration in seconds');
    duration.value = '-4';
    change(duration);
    expect(host.applyComposition).toHaveBeenLastCalledWith({ duration: 0.1 });
    control.destroy();
  });

  it('marks a non-preset size as custom and keeps typed dimensions', () => {
    const host = bridge();
    host.state.w = 1234;
    host.state.h = 567;
    const control = createProjectSettingsControl(host.api);
    const preset = field(control, 'Resolution preset');
    expect(preset.value).toBe('custom');
    expect(preset.querySelector('option[value="custom"]')?.textContent).toBe('Custom · 1234×567');
    control.destroy();
  });

  it('ignores a blank name rather than clearing the project', () => {
    const host = bridge();
    const control = createProjectSettingsControl(host.api);
    const name = field(control, 'Project name');
    name.value = '  ';
    change(name);
    expect(host.applyComposition).not.toHaveBeenCalled();
    expect(name.value).toBe('Reel');
    control.destroy();
  });

  it('writes export defaults through the bridge', () => {
    const host = bridge();
    const control = createProjectSettingsControl(host.api);
    const quality = field(control, 'Export quality');
    quality.value = 'max';
    change(quality);
    expect(host.applyExportDefaults).toHaveBeenCalledWith({ quality: 'max' });
    const format = field(control, 'Export format');
    format.value = 'png';
    change(format);
    const alpha = field(control, 'Transparent background') as HTMLInputElement;
    alpha.checked = true;
    change(alpha);
    expect(host.applyExportDefaults).toHaveBeenLastCalledWith({ alpha: true });
    expect(field(control, 'Export quality').value).toBe('max');
    control.destroy();
  });

  it('dims and locks the export settings the chosen format ignores', () => {
    const host = bridge();
    const control = createProjectSettingsControl(host.api);
    const alpha = field(control, 'Transparent background') as HTMLInputElement;
    const quality = field(control, 'Export quality');
    expect(alpha.disabled).toBe(true);
    expect(quality.disabled).toBe(false);
    const format = field(control, 'Export format');
    format.value = 'still';
    change(format);
    expect(alpha.disabled).toBe(false);
    expect(quality.disabled).toBe(true);
    expect((field(control, 'Export frame rate') as HTMLInputElement).disabled).toBe(true);
    control.destroy();
  });

  it('accepts an arbitrary export size that keeps the aspect', () => {
    const host = bridge();
    const control = createProjectSettingsControl(host.api);
    const scale = field(control, 'Export resolution');
    scale.value = 'custom';
    change(scale);
    const width = field(control, 'Export width in pixels') as HTMLInputElement;
    expect((width.closest('.settings-row') as HTMLElement | null)?.hidden).toBe(false);
    width.value = '1000';
    change(width);
    const written = host.applyExportDefaults.mock.calls.at(-1)![0];
    expect(written.scale).toBeCloseTo(1000 / 1920);
    expect(width.value).toBe('1000');
    expect((field(control, 'Export height in pixels') as HTMLInputElement).value).toBe('562');
    control.destroy();
  });

  it('hides the custom size row while a preset scale is active', () => {
    const host = bridge();
    const control = createProjectSettingsControl(host.api);
    const width = field(control, 'Export width in pixels') as HTMLInputElement;
    expect((width.closest('.settings-row') as HTMLElement | null)?.hidden).toBe(true);
    control.destroy();
  });

  it('labels export resolutions with the composition size', () => {
    const host = bridge();
    const control = createProjectSettingsControl(host.api);
    const labels = [...field(control, 'Export resolution').querySelectorAll('option')]
      .map((option) => option.textContent);
    expect(labels).toEqual(['Half · 960×540', 'Full · 1920×1080', '2× · 3840×2160', 'Custom size…']);
    control.destroy();
  });

  it('uses the host background control when one is provided', () => {
    const swatch = document.createElement('div');
    const host = bridge({ backgroundField: () => swatch });
    const control = createProjectSettingsControl(host.api);
    expect(control.element.contains(swatch)).toBe(true);
    control.destroy();
  });

  it('accepts any frame rate, clamped to a renderable range', () => {
    const host = bridge();
    const control = createProjectSettingsControl(host.api);
    const frameRate = field(control, 'Frame rate');
    frameRate.value = '23';
    change(frameRate);
    expect(host.applyComposition).toHaveBeenCalledWith({ fps: 23 });
    frameRate.value = '900';
    change(frameRate);
    expect(host.applyComposition).toHaveBeenLastCalledWith({ fps: 240 });
    const exportFps = field(control, 'Export frame rate');
    exportFps.value = '48';
    change(exportFps);
    expect(host.applyExportDefaults).toHaveBeenLastCalledWith({ fps: 48 });
    control.destroy();
  });

  it('re-reads the project on refresh after an outside edit', () => {
    const host = bridge();
    const control = createProjectSettingsControl(host.api);
    host.state.fps = 48;
    host.state.name = 'Renamed';
    control.refresh();
    expect(field(control, 'Frame rate').value).toBe('48');
    expect(field(control, 'Project name').value).toBe('Renamed');
    control.destroy();
  });
});

describe('new project form', () => {
  it('uses an in-window background picker without a native colour input', () => {
    let read!: () => string;
    let write!: (value: string) => void;
    const picker = document.createElement('button');
    picker.setAttribute('aria-label', 'Background');
    const backgroundField = vi.fn((get: () => string, set: (value: string) => void) => {
      read = get;
      write = set;
      return picker;
    });
    const form = createNewProjectForm({ bg: '#112233' }, { backgroundField });

    expect(backgroundField).toHaveBeenCalledOnce();
    expect(form.element.contains(picker)).toBe(true);
    expect(form.element.querySelector('input[type="color"]')).toBeNull();
    expect(read()).toBe('#112233');

    write('#aabbcc');
    expect(form.values().bg).toBe('#AABBCC');
  });

  it('returns the chosen composition settings', () => {
    const form = createNewProjectForm();
    const preset = form.element.querySelector<HTMLSelectElement>('[aria-label="Resolution preset"]')!;
    preset.value = '1080x1080';
    change(preset);
    const duration = form.element.querySelector<HTMLInputElement>('[aria-label="Duration in seconds"]')!;
    duration.value = '4.5';
    const frameRate = form.element.querySelector<HTMLInputElement>('[aria-label="Frame rate"]')!;
    frameRate.value = '48';
    const name = form.element.querySelector<HTMLInputElement>('[aria-label="Project name"]')!;
    name.value = '  Launch  ';
    expect(form.values()).toEqual({ name: 'Launch', w: 1080, h: 1080, fps: 48, dur: 4.5, bg: '#09090A' });
  });

  it('starts from the values it is seeded with', () => {
    const form = createNewProjectForm({ w: 1080, h: 1920, fps: 60, dur: 6, bg: '#112233' });
    expect(form.values()).toMatchObject({ w: 1080, h: 1920, fps: 60, dur: 6, bg: '#112233' });
    const preset = form.element.querySelector<HTMLSelectElement>('[aria-label="Resolution preset"]')!;
    expect(preset.value).toBe('1080x1920');
    const frameRate = form.element.querySelector<HTMLInputElement>('[aria-label="Frame rate"]')!;
    expect(frameRate.value).toBe('60');
  });

  it('falls back to safe values when the fields are emptied', () => {
    const form = createNewProjectForm();
    const width = form.element.querySelector<HTMLInputElement>('[aria-label="Width in pixels"]')!;
    width.value = '';
    const name = form.element.querySelector<HTMLInputElement>('[aria-label="Project name"]')!;
    name.value = '';
    expect(form.values()).toMatchObject({ name: 'Untitled', w: 1920 });
  });
});

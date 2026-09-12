import { expect, it } from 'vitest';
import { PANEL_ICONS, panelIcons } from './panel-icons';
import { NATIVE_PANEL_DESIGN } from './agent/panel-focus';

it('gives all existing panels distinct meaningful icons', () => {
  const icons = Object.fromEntries(Object.values(PANEL_ICONS).map(icon => [icon, '<svg/>']));
  const result = panelIcons(Object.keys(PANEL_ICONS).map(id => ({ id })), icons);
  expect(new Set(Object.values(result)).size).toBe(Object.keys(PANEL_ICONS).length);
  expect(Object.values(result)).not.toContain('panel');
});

it('respects authored icons and replaces unknown or duplicate icons consistently', () => {
  const panels = [{ id: 'custom-a', icon: 'graph' }, { id: 'custom-b', icon: 'invalid' }, { id: 'custom-c', icon: 'graph' }];
  const icons = { graph: '<svg/>', clock: '<svg/>', cam: '<svg/>' };
  const result = panelIcons(panels, icons);
  expect(result['custom-a']).toBe('graph');
  expect(new Set(Object.values(result)).size).toBe(3);
  expect(panelIcons([...panels].reverse(), icons)).toEqual(result);
  expect(NATIVE_PANEL_DESIGN).toContain('Choose it yourself');
  expect(NATIVE_PANEL_DESIGN).toContain('section.icon');
});

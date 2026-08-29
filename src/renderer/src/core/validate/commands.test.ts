/// <reference types="node" />

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

import { assertNever, COMMAND_TYPES, type EditCommand } from '../types/commands';
import {
  MAX_DELETE_TARGETS,
  MAX_EASING_TARGETS,
  MAX_KEYFRAMES,
  ValidationError,
  parseAgentEditCommand,
  parseEditCommand,
  parseEditCommands
} from './commands';

// The legacy oracle file is gone (Phase 6); the shared vocabulary is now the
// canonical source, itself pinned against Edit.operations by the transplanted
// contracts suite (src/renderer/src/legacy/__tests__).
import { EDIT_COMMAND_TYPES } from '../../../../shared/edit-vocabulary';

function legacyGoldenCommandTypes(): string[] {
  return [...EDIT_COMMAND_TYPES];
}

const channel = (value: number) => ({ v: value, kf: [], expr: null });
const serializedTextLayer = {
  id: 'serialized-layer', type: 'text', name: 'Title', from: 0, dur: 5,
  on: true, lock: false, shy: false, collapsed: true,
  color: '#E8E2CF', blend: 'normal', mblur: false, parent: null,
  p: {
    'anchor.x': channel(0), 'anchor.y': channel(0),
    'position.x': channel(960), 'position.y': channel(540),
    'scale.x': channel(100), 'scale.y': channel(100),
    rotation: channel(0), opacity: channel(100), skew: channel(0)
  },
  fx: [], masks: [], locked_intent: {},
  d: {
    text: 'Powermove', font: 'SF Pro Display', weight: 600, size: 128, tracking: -2,
    leading: 1.1, color: '#F2F2F2', align: 'center', italic: false
  }
};

const validCommands: unknown[] = [
  { type: 'set_property', target: 'title', path: 'position.x', value: 42, time: '1.5', mode: 'auto' },
  {
    type: 'replace_keyframes', target: '$selection', path: 'opacity',
    keyframes: [{ time: '0', value: 0 }, { time: 1, value: 100, ease: 'linear', hold: false }]
  },
  { type: 'set_easing', keyframes: ['k1', 'k2'], curve: ['.2', '.1', '.8', '.9'] },
  { type: 'set_expression', target: 1, path: 'opacity', expression: 'value + 1' },
  { type: 'set_content', target: 'title', patch: { text: 'Hello' } },
  { type: 'set_layer', target: null, patch: { name: 'Hero', from: '1', visible: 1 } },
  { type: 'set_composition', patch: { width: '1920', workArea: ['0', '10'] } },
  { type: 'add_layer', id: 'new', layerType: 'solid', content: { color: '#112233' }, select: false },
  { type: 'delete_layers', targets: ['a', 'b'] },
  { type: 'reorder_layer', target: 'a', index: '2' },
  { type: 'add_effect', target: 'a', effect: 'blur', parameters: { amount: 12 }, open: true, enabled: false },
  { type: 'remove_effect', target: 'a', effect: 'blur' },
  { type: 'set_effect', target: 'a', effect: 'blur', patch: { enabled: 0, open: 1 } },
  {
    type: 'set_transition', layer: 'a', edge: 'in',
    transition: { type: 'wipe', dur: '0.5', p: { angle: 45, tint: '#112233', reverse: false } }
  },
  { type: 'set_scene_parameter', name: 'Intensity', value: 42, min: '0', max: '100' },
  { type: 'add_marker', id: 'm1', time: '2.5', name: 'Beat' },
  { type: 'create_section', section: { id: 'section-1', layers: [], versions: [] } },
  {
    type: 'update_section', sectionId: 'section-1', layers: [serializedTextLayer],
    version: { id: 'version-1', layers: [serializedTextLayer], at: '1' }
  },
  {
    type: 'transform_layers',
    transform: {
      version: 99,
      selector: { scope: 'all', includeLocked: true, types: ['text', 'not-a-layer'] },
      order: 'reverseStack',
      edits: [{ path: 'properties.opacity', value: { op: 'multiply', args: [{ ref: 'current' }, 0.5] } }]
    },
    state: { scale: 2 }
  }
];

const invalidCommands: unknown[] = [
  { type: 'set_property', value: 42 },
  { type: 'replace_keyframes', path: 'opacity', keyframes: 'not-an-array' },
  { type: 'set_easing', keyframes: [], curve: [0, 0, 1, 1] },
  { type: 'set_expression', expression: 'value' },
  { type: 'set_content', patch: [] },
  { type: 'set_layer', patch: { notEditable: true } },
  { type: 'set_composition', patch: { notEditable: true } },
  { type: 'add_layer', layerType: 'bogus' },
  { type: 'delete_layers', targets: [{}] },
  { type: 'reorder_layer', index: Number.NaN },
  { type: 'add_effect', effect: '' },
  { type: 'remove_effect' },
  { type: 'set_effect', effect: 'blur', patch: { amount: 12 } },
  { type: 'set_transition', layer: 'a', edge: 'middle', transition: null },
  { type: 'set_scene_parameter', name: '', value: 1 },
  { type: 'add_marker', time: Number.POSITIVE_INFINITY },
  { type: 'create_section', section: {} },
  { type: 'update_section', sectionId: 'section-1', layers: [] },
  { type: 'transform_layers', transform: { edits: [] } }
];

const huge = 'x'.repeat(50_001);
const oversizedCommands: unknown[] = [
  { type: 'set_property', path: 'opacity', value: huge },
  { type: 'replace_keyframes', path: 'opacity', keyframes: [{ time: 0, value: 0 }], expression: huge },
  { type: 'set_easing', keyframes: [huge], curve: [0, 0, 1, 1] },
  { type: 'set_expression', path: 'opacity', expression: huge },
  { type: 'set_content', patch: { text: huge } },
  { type: 'set_layer', patch: { name: huge } },
  { type: 'set_composition', patch: { name: huge } },
  { type: 'add_layer', layerType: 'solid', name: huge },
  { type: 'delete_layers', targets: [huge] },
  { type: 'reorder_layer', target: huge, index: 0 },
  { type: 'add_effect', effect: 'blur', parameters: { label: huge } },
  { type: 'remove_effect', effect: huge },
  { type: 'set_effect', effect: huge, patch: {} },
  { type: 'set_transition', layer: 'a', edge: 'in', transition: { type: huge } },
  { type: 'set_scene_parameter', name: 'Value', value: huge },
  { type: 'add_marker', name: huge },
  { type: 'create_section', section: { id: 'section', note: huge } },
  { type: 'update_section', sectionId: 'section', layers: [serializedTextLayer], thumb: huge },
  {
    type: 'transform_layers',
    transform: { edits: [{ path: 'properties.opacity', value: 1 }], note: huge }
  }
];

function exhaustivelyName(command: EditCommand): string {
  switch (command.type) {
    case 'set_property': return command.type;
    case 'replace_keyframes': return command.type;
    case 'set_easing': return command.type;
    case 'set_expression': return command.type;
    case 'set_content': return command.type;
    case 'set_layer': return command.type;
    case 'set_composition': return command.type;
    case 'add_layer': return command.type;
    case 'delete_layers': return command.type;
    case 'reorder_layer': return command.type;
    case 'add_effect': return command.type;
    case 'remove_effect': return command.type;
    case 'set_effect': return command.type;
    case 'set_transition': return command.type;
    case 'set_scene_parameter': return command.type;
    case 'add_marker': return command.type;
    case 'create_section': return command.type;
    case 'update_section': return command.type;
    case 'transform_layers': return command.type;
    default: return assertNever(command);
  }
}

describe('EditCommand contract', () => {
  it('matches the shared operation vocabulary in contract order', () => {
    expect(COMMAND_TYPES).toEqual(legacyGoldenCommandTypes());
  });

  it('supports an exhaustive discriminated-union switch', () => {
    for (const raw of validCommands) {
      const parsed = parseEditCommand(raw);
      expect(parsed).not.toBeInstanceOf(ValidationError);
      if (!(parsed instanceof ValidationError)) expect(exhaustivelyName(parsed)).toBe(parsed.type);
    }
  });
});

describe('parseEditCommand', () => {
  it('accepts one canonical example of every operation', () => {
    expect(validCommands).toHaveLength(COMMAND_TYPES.length);
    expect(validCommands.map(parseEditCommand).filter(value => value instanceof ValidationError)).toEqual([]);
  });

  it('rejects one malformed example of every operation', () => {
    expect(invalidCommands).toHaveLength(COMMAND_TYPES.length);
    for (const raw of invalidCommands) expect(parseEditCommand(raw)).toBeInstanceOf(ValidationError);
  });

  it('rejects an oversized known-field payload for every operation', () => {
    expect(oversizedCommands).toHaveLength(COMMAND_TYPES.length);
    for (const raw of oversizedCommands) expect(parseEditCommand(raw)).toBeInstanceOf(ValidationError);
  });

  it('strips unknown top-level fields for every operation', () => {
    for (const raw of validCommands) {
      const withUnknown = { ...(raw as Record<string, unknown>), unknownField: 'strip me' };
      const parsed = parseEditCommand(withUnknown);
      expect(parsed).not.toBeInstanceOf(ValidationError);
      if (!(parsed instanceof ValidationError)) {
        expect(Object.hasOwn(parsed, 'unknownField')).toBe(false);
      }
    }
  });

  it('strips unsafe command-level keys for every operation without prototype pollution', () => {
    const unsafe = JSON.parse('{"__proto__":{"polluted":true}}') as Record<string, unknown>;
    for (const raw of validCommands) {
      const parsed = parseEditCommand({ ...unsafe, ...(raw as Record<string, unknown>) });
      expect(parsed).not.toBeInstanceOf(ValidationError);
      if (!(parsed instanceof ValidationError)) expect(Object.hasOwn(parsed, '__proto__')).toBe(false);
      expect((Object.prototype as { polluted?: boolean }).polluted).toBeUndefined();
    }
  });

  it('applies legacy numeric and boolean coercion', () => {
    const property = parseEditCommand(validCommands[0]);
    expect(property).toMatchObject({ time: 1.5 });

    const layer = parseEditCommand(validCommands[5]);
    expect(layer).toMatchObject({ patch: { from: 1, visible: true } });

    const composition = parseEditCommand(validCommands[6]);
    expect(composition).toMatchObject({ patch: { width: 1920, workArea: [0, 10] } });
  });

  it('preserves editor lock and hand-intent fields without rewriting them', () => {
    const property = parseEditCommand({
      type: 'set_property', target: 'L-1', path: 'position.x', value: 1040,
      mode: 'static', preserveHandEdits: false, markIntent: 'human', overrideLock: true
    });
    expect(property).toEqual({
      type: 'set_property', target: 'L-1', path: 'position.x', value: 1040,
      mode: 'static', preserveHandEdits: false, markIntent: 'human', overrideLock: true
    });

    expect(parseEditCommand({
      type: 'replace_keyframes', path: 'opacity', keyframes: [],
      preserveHandEdits: false, overrideLock: true
    })).toMatchObject({ preserveHandEdits: false, overrideLock: true });
    expect(parseEditCommand({
      type: 'set_content', patch: { text: 'Unlocked' }, overrideLock: true
    })).toMatchObject({ overrideLock: true });
  });

  it('keeps the agent allowlist separate and forces hand-intent protection', () => {
    const parsed = parseAgentEditCommand({
      type: 'set_property', target: 'L-1', path: 'position.x', value: 1040,
      preserveHandEdits: false, markIntent: 'human', overrideLock: true
    });
    expect(parsed).toEqual({
      type: 'set_property', target: 'L-1', path: 'position.x', value: 1040,
      preserveHandEdits: true
    });
    expect(parseAgentEditCommand({ type: 'create_section', section: { id: 'section-1' } }))
      .toBeInstanceOf(ValidationError);
    expect(parseAgentEditCommand({
      type: 'set_transition', layer: 'L-1', edge: 'out', transition: null
    })).toEqual({ type: 'set_transition', layer: 'L-1', edge: 'out', transition: null });
  });

  it('rejects the removed Solo layer feature', () => {
    expect(parseEditCommand({ type: 'set_layer', patch: { solo: true } })).toBeInstanceOf(ValidationError);
    expect(parseEditCommand({ type: 'add_layer', layerType: 'solid', solo: true })).toEqual({
      type: 'add_layer', layerType: 'solid'
    });
  });

  it('validates transition duration and static parameter values', () => {
    expect(parseEditCommand(validCommands[13])).toEqual({
      type: 'set_transition', layer: 'a', edge: 'in',
      transition: { type: 'wipe', dur: 0.5, p: { angle: 45, tint: '#112233', reverse: false } }
    });
    for (const transition of [
      { type: 'wipe', dur: 0.019 },
      { type: 'wipe', dur: 601 },
      { type: 'wipe', dur: Number.NaN },
      { type: 'wipe', p: { tint: 'red' } },
      { type: 'wipe', p: { amount: Number.POSITIVE_INFINITY } },
      { type: 'wipe', p: { nested: { nope: true } } }
    ]) {
      expect(parseEditCommand({ type: 'set_transition', layer: null, edge: 'in', transition }))
        .toBeInstanceOf(ValidationError);
    }
  });

  it('caps model-authored keyframes and delete targets', () => {
    const keyframes = parseEditCommand({
      type: 'replace_keyframes', path: 'opacity',
      keyframes: Array.from({ length: 85 }, (_, index) => ({ time: index, value: index }))
    });
    expect(keyframes).not.toBeInstanceOf(ValidationError);
    if (!(keyframes instanceof ValidationError) && keyframes.type === 'replace_keyframes') {
      expect(keyframes.keyframes).toHaveLength(MAX_KEYFRAMES);
    }

    const deleted = parseEditCommand({
      type: 'delete_layers', targets: Array.from({ length: 25 }, (_, index) => `layer-${index}`)
    });
    expect(deleted).not.toBeInstanceOf(ValidationError);
    if (!(deleted instanceof ValidationError) && deleted.type === 'delete_layers') {
      expect(deleted.targets).toHaveLength(MAX_DELETE_TARGETS);
    }
  });

  it('allows the legacy 1000-keyframe easing target cap', () => {
    const parsed = parseEditCommand({
      type: 'set_easing',
      keyframes: Array.from({ length: MAX_EASING_TARGETS + 5 }, (_, index) => `key-${index}`),
      curve: 'power'
    });
    expect(parsed).not.toBeInstanceOf(ValidationError);
    if (!(parsed instanceof ValidationError) && parsed.type === 'set_easing') {
      expect(parsed.keyframes).toHaveLength(MAX_EASING_TARGETS);
    }
  });

  it('enforces the cleaner JSON-size limit before and after field stripping', () => {
    expect(parseEditCommand(JSON.stringify({ type: 'set_expression', path: 'opacity', expression: huge })))
      .toBeInstanceOf(ValidationError);
    expect(parseEditCommand({ type: 'set_expression', path: 'opacity', expression: huge }))
      .toBeInstanceOf(ValidationError);

    /* cleanCommand measures parsed output, so a large unknown field is harmless. */
    const stripped = parseEditCommand({ type: 'add_marker', unknownField: huge });
    expect(stripped).toEqual({ type: 'add_marker' });
  });

  it('rejects every forbidden safePatch key without polluting prototypes', () => {
    const patchCommands = [
      ['set_content', 'patch'],
      ['set_layer', 'patch'],
      ['set_composition', 'patch'],
      ['add_layer', 'content'],
      ['add_layer', 'properties'],
      ['add_effect', 'parameters'],
      ['set_effect', 'patch'],
      ['set_transition', 'transition.p']
    ] as const;
    for (const [type, field] of patchCommands) {
      for (const unsafe of ['__proto__', 'prototype', 'constructor']) {
        const patch = JSON.parse(`{"${unsafe}":{"polluted":true}}`) as Record<string, unknown>;
        const base = type === 'add_layer' ? { type, layerType: 'solid' }
          : type === 'add_effect' ? { type, effect: 'blur' }
            : type === 'set_effect' ? { type, effect: 'blur' }
              : type === 'set_transition'
                ? { type, layer: 'a', edge: 'in', transition: { type: 'wipe', p: patch } }
              : { type };
        const command = type === 'set_transition' ? base : { ...base, [field]: patch };
        expect(parseEditCommand(command)).toBeInstanceOf(ValidationError);
        expect((Object.prototype as { polluted?: boolean }).polluted).toBeUndefined();
      }
    }
  });

  it('sanitizes the recursive transform grammar', () => {
    const parsed = parseEditCommand(validCommands.at(-1));
    expect(parsed).not.toBeInstanceOf(ValidationError);
    if (!(parsed instanceof ValidationError) && parsed.type === 'transform_layers') {
      expect(parsed.transform).toMatchObject({
        version: 1,
        selector: { scope: 'all', includeLocked: false, types: ['text'] },
        order: 'reverseStack'
      });
    }
  });

  it('omits an invalid state fallback while preserving the transform edit', () => {
    const parsed = parseEditCommand({
      type: 'transform_layers',
      transform: {
        edits: [{ path: 'layer.from', value: { state: 'offset', fallback: { nope: 1 } } }]
      }
    });
    expect(parsed).not.toBeInstanceOf(ValidationError);
    if (!(parsed instanceof ValidationError) && parsed.type === 'transform_layers') {
      expect(parsed.transform.edits[0]?.value).toEqual({ state: 'offset' });
    }
  });
});

describe('parseEditCommands', () => {
  it('bounds lists and returns only typed commands', () => {
    const parsed = parseEditCommands(validCommands, { maxCommands: 3 });
    expect(parsed).not.toBeInstanceOf(ValidationError);
    if (!(parsed instanceof ValidationError)) expect(parsed.map(command => command.type)).toEqual(COMMAND_TYPES.slice(0, 3));
  });

  it('skips invalid list members by default like sanitizeProposal', () => {
    const parsed = parseEditCommands([validCommands[0], { type: 'run_shell' }]);
    expect(parsed).not.toBeInstanceOf(ValidationError);
    if (!(parsed instanceof ValidationError)) expect(parsed.map(command => command.type)).toEqual(['set_property']);
  });

  it('can reject an entire batch and reports the invalid member index', () => {
    const parsed = parseEditCommands(
      [validCommands[0], { type: 'run_shell' }],
      { onInvalid: 'reject' }
    );
    expect(parsed).toBeInstanceOf(ValidationError);
    expect((parsed as ValidationError).message).toContain('commands[1]');
  });

  it('uses the agent command contract when requested', () => {
    const parsed = parseEditCommands([
      { type: 'set_property', path: 'opacity', value: 20, preserveHandEdits: false },
      { type: 'create_section', section: { id: 'skip-me' } }
    ], { source: 'agent' });
    expect(parsed).toEqual([{
      type: 'set_property', path: 'opacity', value: 20, preserveHandEdits: true
    }]);
  });
});

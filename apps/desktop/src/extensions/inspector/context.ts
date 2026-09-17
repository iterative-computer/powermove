import { getContext, onDestroy, setContext } from 'svelte';
import type {
  ControlEditBinding,
  EditCommand,
  EditMeta,
  EditResult,
  InspectorService,
  Layer,
  PowermoveAPI,
  TimelineService,
  ToolService,
  ViewerService
} from 'powermove';
import { createInspectorEdit, inspectorMixed } from './multi-edit';
import { createInspectorSignal } from './api-signal.svelte.js';

const INSPECTOR_CONTEXT = Symbol('powermove.inspector');

export type EditBinding = ControlEditBinding;
export type SelectOption = string | { v: unknown; label: string };

type TickKind = 'values' | 'structure' | 'project' | 'assets' | 'library' | 'history';

export interface InspectorDocumentState {
  readonly proj: ReturnType<PowermoveAPI['project']['get']>;
  readonly tick: Record<TickKind, number>;
  bump(...scopes: TickKind[]): void;
}

export interface InspectorSelectionState {
  readonly layers: string[];
  readonly keys: string[];
  readonly chan: string | null;
}

export interface InspectorTransportState {
  readonly time: number;
}

export interface ShaderParameterDefinition {
  name: string;
  type: string;
  label: string;
  control: 'color' | 'toggle' | 'num';
  def: unknown;
  min?: number;
  max?: number;
}

export interface InspectorRuntimeService extends InspectorService {
  setEffectSelection(layerId: string, ids: string[]): void;
  clearEffectSelection(): void;
  syncShaderUniforms(layer: Layer): void;
  shaderDefinitions(layer: Layer): ShaderParameterDefinition[];
}

export interface InspectorViewerState extends ViewerService {
  activePath?: string | null;
  textSelection?: { layer: string; start: number; end: number } | null;
}

export interface InspectorEditAPI {
  apply(input: EditCommand | EditCommand[], meta?: EditMeta): EditResult;
  begin(label: string, meta?: EditMeta): void;
  dispatch(input: EditCommand | EditCommand[]): EditResult;
  commit(label?: string): EditResult;
  cancel(): boolean;
  mutate<T>(label: string, action: () => T, meta?: EditMeta): T | EditResult;
}

type InspectorTimelineService = TimelineService & { focusGraph?(layer: Layer, channel: string): void };

export interface InspectorContext {
  api: PowermoveAPI;
  doc: InspectorDocumentState;
  sel: InspectorSelectionState;
  transport: InspectorTransportState;
  edit: InspectorEditAPI;
  mixed(binding: EditBinding, value: unknown): boolean;
  inspector(): InspectorRuntimeService | null;
  timeline(): InspectorTimelineService | null;
  tools(): ToolService | null;
  viewer(): InspectorViewerState | null;
}

function createReactiveState(api: PowermoveAPI): Pick<InspectorContext, 'doc' | 'sel' | 'transport'> {
  const counts: Record<TickKind, number> = {
    values: 0,
    structure: 0,
    project: 0,
    assets: 0,
    library: 0,
    history: 0
  };
  const signal = createInspectorSignal();
  const subscriptions = [
    api.events.on('project:changed', ({ kind }) => {
      if (kind === 'replace') for (const key of Object.keys(counts) as TickKind[]) counts[key]++;
      else counts[kind as TickKind]++;
      signal.bump();
    }),
    api.events.on('selection', () => signal.bump()),
    api.events.on('time', () => signal.bump()),
    api.events.on('transport', () => signal.bump()),
    // Host UI invalidation (stopwatch toggles, reveal, restores) repaints panels.
    api.events.on('invalidate', (what) => { if (what === 'ui') signal.bump(); })
  ];
  onDestroy(() => {
    for (const subscription of subscriptions) subscription.dispose();
  });
  const tick = {} as Record<TickKind, number>;
  for (const key of Object.keys(counts) as TickKind[]) {
    Object.defineProperty(tick, key, { enumerable: true, get: () => { signal.version; return counts[key]; } });
  }
  return {
    doc: {
      get proj() { signal.version; return api.project.get(); },
      tick,
      bump(...scopes) {
        for (const scope of (scopes.length ? scopes : ['values'] as TickKind[])) counts[scope]++;
        signal.bump();
      }
    },
    sel: {
      get layers() { signal.version; return api.selection.layers(); },
      get keys() { signal.version; return api.selection.keys(); },
      get chan() { signal.version; return api.selection.chan(); }
    },
    transport: {
      get time() { signal.version; return api.transport.time(); }
    }
  };
}

export function provideInspectorContext(api: PowermoveAPI): InspectorContext {
  const state = createReactiveState(api);
  const edit = createInspectorEdit(api);
  /*
   * Controls build their own EditGesture from the `api` they are handed, so the
   * inspector hands them one whose `edit` already fans a change out over the
   * selection. Without this every field would write to the primary layer alone
   * while still rendering "Mixed" for the rest.
   */
  const editingApi: PowermoveAPI = { ...api, edit };
  const context: InspectorContext = {
    api: editingApi,
    ...state,
    edit,
    mixed: (binding, value) => inspectorMixed(api, binding, value),
    inspector: () => api.services.get<InspectorRuntimeService>('inspector'),
    timeline: () => api.services.get<InspectorTimelineService>('timeline'),
    tools: () => api.services.get<ToolService>('tool'),
    viewer: () => api.services.get<InspectorViewerState>('viewer')
  };
  setContext(INSPECTOR_CONTEXT, context);
  return context;
}

export function inspectorContext(): InspectorContext {
  return getContext<InspectorContext>(INSPECTOR_CONTEXT);
}

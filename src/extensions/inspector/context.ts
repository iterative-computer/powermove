import { getContext, setContext } from 'svelte';
import type { ControlEditBinding, PowermoveAPI } from 'powermove';

const INSPECTOR_CONTEXT = Symbol('powermove.inspector');

export type EditBinding = ControlEditBinding;
export type SelectOption = string | { v: unknown; label: string };

export interface InspectorDocumentState {
  proj: any;
  tick: Record<'values' | 'structure' | 'project' | 'assets' | 'library' | 'history', number>;
  bump(...scopes: Array<'values' | 'structure' | 'project' | 'assets' | 'library' | 'history'>): void;
  replace(next: any): void;
}

export interface InspectorSelectionState {
  layers: string[];
  keys: string[];
  chan: string | null;
}

export interface InspectorTransportState {
  time: number;
  playing: boolean;
  quality: number;
  loop: boolean;
  snap: boolean;
  tool: string;
}

export interface InspectorContext {
  api: PowermoveAPI;
  PM: Record<string, any>;
  doc: InspectorDocumentState;
  sel: InspectorSelectionState;
  transport: InspectorTransportState;
}

export function provideInspectorContext(api: PowermoveAPI): InspectorContext {
  const state = api.host.state as {
    doc: InspectorDocumentState;
    sel: InspectorSelectionState;
    transport: InspectorTransportState;
  };
  const context = {
    api,
    PM: api.host.pm as Record<string, any>,
    doc: state.doc,
    sel: state.sel,
    transport: state.transport
  };
  setContext(INSPECTOR_CONTEXT, context);
  return context;
}

export function inspectorContext(): InspectorContext {
  return getContext<InspectorContext>(INSPECTOR_CONTEXT);
}

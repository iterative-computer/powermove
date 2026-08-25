import { getContext, setContext } from 'svelte';

const ROW_LABEL = Symbol('powermove-control-row-label');

export function provideRowLabel(id: string): void {
  setContext(ROW_LABEL, id);
}

export function rowLabelId(): string | undefined {
  return getContext<string | undefined>(ROW_LABEL);
}


/* Shared state between the FX browser's header (segmented control + search
   toggle, mounted by the panel header hook) and its body (the list). */
export type FxKind = 'effect' | 'transition';

export const fxBrowser = $state({
  kind: 'effect' as FxKind,
  query: '',
  searchOpen: false
});

export const FX_KIND_OPTIONS = [
  { id: 'effect', label: 'Effects' },
  { id: 'transition', label: 'Transitions' }
];

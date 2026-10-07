import type { Disposable, PowermoveAPI } from 'powermove';

/*
 * Reactive document and playhead values for contributed 3D inspector fields.
 * Playback refreshes at most 15 times a second, like the native inspector.
 */
export interface SceneUiState {
  readonly version: number;
  readonly time: number;
  dispose(): void;
}

export function createSceneUiState(api: PowermoveAPI): SceneUiState {
  let version = $state(0);
  let time = $state(api.transport.time());
  let lastTime = -Infinity;
  const bump = () => { version++; };
  const subscriptions: Disposable[] = [
    api.events.on('project:changed', bump),
    api.events.on('selection', bump),
    api.events.on('invalidate', (what) => { if (what === 'ui') bump(); }),
    api.events.on('transport', () => { time = api.transport.time(); }),
    api.events.on('time', (next) => {
      const now = performance.now();
      if (!api.transport.playing() || now - lastTime >= 1000 / 15) { lastTime = now; time = next; }
    })
  ];
  return {
    get version() { return version; },
    get time() { return time; },
    dispose() { for (const subscription of subscriptions.splice(0)) subscription.dispose(); }
  };
}

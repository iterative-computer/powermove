import { expect, launchApp, test } from './helpers/app';

/** With Use New Engine off, opened projects restore embedded media into the
 * page's IndexedDB store. A large project's open reply can block the page for
 * over a minute; the store's first open must not count that stall against
 * its timeout, or the restore fails and the project opens blank
 * (docs/engine/host-processes.md). Each launch uses and removes its own
 * temporary profile. */
test('the legacy media store survives a long main-thread task during its first open', async () => {
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '0' } });
  try {
    await session.openEditor();
    const result = await session.page.evaluate(async () => {
      const PM = (window as any).PM;
      if (PM.HostMedia?.enabled) throw new Error('expected the legacy media store (Use New Engine off)');
      // The first access opens the database...
      const first = PM.MediaStore.put('long-task-first', new Blob(['first'], { type: 'text/plain' }));
      // ...and the page blocks before its answer can be handled.
      const until = performance.now() + 8000;
      while (performance.now() < until) { /* a long main-thread task */ }
      // A project restore then writes while that open is still settling.
      const second = PM.MediaStore.put('long-task-second', new Blob(['second'], { type: 'text/plain' }));
      const stored = await Promise.all([first, second]);
      const read = await PM.MediaStore.get('long-task-second');
      await Promise.all([PM.MediaStore.remove('long-task-first'), PM.MediaStore.remove('long-task-second')]);
      return { stored, read: read ? await read.text() : null };
    });
    expect(result).toEqual({ stored: [true, true], read: 'second' });
  } finally {
    await session.close();
  }
});

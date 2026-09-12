import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => {
  await session.page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Pro shortcut proof', w: 640, h: 360, dur: 8, fps: 30 });
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
    for (const [index, name] of ['Back', 'Middle', 'Front'].entries()) {
      const result = PM.Edit.apply({
        type: 'add_layer', layerType: 'solid', name, from: 0, duration: 8, index,
        properties: { 'position.x': 100 + index * 100, 'position.y': 180 },
      }, { label: `Add ${name}`, origin: 'test' });
      if (!result.ok) throw new Error(result.message);
    }
    PM.hist.clear();
    PM.setTime(4);
    PM.selectLayers([PM.proj.layers[1].id]);
  });
});

test('Command+Shift+D splits a group into two group strips', async ({ session }) => {
  const { app, page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const group = PM.groupLayers(PM.proj.layers.slice(0, 2).map((layer: any) => layer.id), 'Pair');
    PM.hist.clear();
    PM.selectLayers(group.id);
  });

  await app.evaluate(({ BrowserWindow }) => {
    const contents = BrowserWindow.getAllWindows()[0]?.webContents;
    if (!contents) throw new Error('Missing editor window');
    const primary: 'command' | 'control' = process.platform === 'darwin' ? 'command' : 'control';
    contents.sendInputEvent({ type: 'keyDown', keyCode: 'd', modifiers: [primary, 'shift'] });
    contents.sendInputEvent({ type: 'keyUp', keyCode: 'd', modifiers: [primary, 'shift'] });
  });

  await expect.poll(() => page.evaluate(() => {
    const PM = (window as any).PM;
    const groups = PM.proj.layers.filter((layer: any) => layer.type === 'group');
    return {
      groups: groups.length,
      selectedType: PM.firstSel()?.type,
      memberCounts: groups.map((group: any) => PM.proj.layers.filter((layer: any) => layer.group === group.id).length),
      history: PM.hist.list(),
    };
  })).toEqual({ groups: 2, selectedType: 'group', memberCounts: [2, 2], history: ['Split'] });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('professional shortcuts split, cut, paste, nudge, and respect focused fields', async ({ session }) => {
  const { app, page } = session;
  await page.locator('body').click({ position: { x: 20, y: 100 } });

  await app.evaluate(({ BrowserWindow }) => {
    const contents = BrowserWindow.getAllWindows()[0]?.webContents;
    if (!contents) throw new Error('Missing editor window');
    const primary: 'command' | 'control' = process.platform === 'darwin' ? 'command' : 'control';
    const modifiers: Array<'command' | 'control' | 'shift'> = [primary, 'shift'];
    contents.sendInputEvent({ type: 'keyDown', keyCode: 'd', modifiers });
    contents.sendInputEvent({ type: 'keyUp', keyCode: 'd', modifiers });
  });
  let state = await page.evaluate(() => {
    const PM = (window as any).PM;
    const selected = PM.firstSel();
    return {
      count: PM.proj.layers.length,
      selectedFrom: selected?.from,
      selectedDuration: selected?.dur,
      history: PM.hist.list(),
    };
  });
  expect(state).toEqual({ count: 4, selectedFrom: 4, selectedDuration: 4, history: ['Split'] });

  await page.keyboard.press('Meta+Z');
  expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(3);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.selectLayers([PM.proj.layers[1].id]);
  });
  await page.keyboard.press('Meta+X');
  expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(2);
  await page.keyboard.press('Meta+V');
  state = await page.evaluate(() => {
    const PM = (window as any).PM;
    return { count: PM.proj.layers.length, selected: PM.sel.layers.length, history: PM.hist.list().slice(-2) };
  });
  expect(state).toEqual({ count: 3, selected: 1, history: ['Cut layers', 'Paste layers'] });

  const before = await page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.ev(PM.firstSel(), 'position.x', PM.time);
  });
  await page.keyboard.press('ArrowRight');
  await page.waitForTimeout(220);
  expect(await page.evaluate(() => {
    const PM = (window as any).PM;
    return PM.ev(PM.firstSel(), 'position.x', PM.time);
  })).toBe(before + 1);

  await page.evaluate(() => {
    const input = document.createElement('input');
    input.id = 'pro-shortcut-field';
    input.value = 'field';
    document.body.append(input);
    input.focus();
  });
  await app.evaluate(({ BrowserWindow }) => {
    const contents = BrowserWindow.getAllWindows()[0]?.webContents;
    if (!contents) throw new Error('Missing editor window');
    const primary: 'command' | 'control' = process.platform === 'darwin' ? 'command' : 'control';
    contents.sendInputEvent({ type: 'keyDown', keyCode: 'd', modifiers: [primary, 'shift'] });
    contents.sendInputEvent({ type: 'keyUp', keyCode: 'd', modifiers: [primary, 'shift'] });
  });
  const fieldResult = await page.evaluate(() => {
    const PM = (window as any).PM;
    const input = document.getElementById('pro-shortcut-field');
    const result = { count: PM.proj.layers.length, focused: document.activeElement === input };
    input?.remove();
    return result;
  });
  expect(fieldResult).toEqual({ count: 3, focused: true });

  await page.locator('body').click({ position: { x: 20, y: 100 } });
  await app.evaluate(({ BrowserWindow }) => {
    const contents = BrowserWindow.getAllWindows()[0]?.webContents;
    if (!contents) throw new Error('Missing editor window');
    const primary: 'command' | 'control' = process.platform === 'darwin' ? 'command' : 'control';
    contents.sendInputEvent({ type: 'keyDown', keyCode: 'd', modifiers: [primary, 'shift'] });
    contents.sendInputEvent({ type: 'keyUp', keyCode: 'd', modifiers: [primary, 'shift'] });
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(4);

  await app.evaluate(({ BrowserWindow }) => {
    const contents = BrowserWindow.getAllWindows()[0]?.webContents;
    if (!contents) throw new Error('Missing editor window');
    const primary: 'command' | 'control' = process.platform === 'darwin' ? 'command' : 'control';
    contents.sendInputEvent({ type: 'keyDown', keyCode: 'h', modifiers: [primary, 'shift'] });
    contents.sendInputEvent({ type: 'keyDown', keyCode: 'h', modifiers: [primary, 'shift', 'isautorepeat'] });
    contents.sendInputEvent({ type: 'keyUp', keyCode: 'h', modifiers: [primary, 'shift'] });
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.Viewer.showControls)).toBe(false);
  expect(await page.evaluate(() => (window as any).PM.firstSel()?.on)).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('repeat safety and native menus use the same context-aware editor commands', async ({ session }) => {
  const { app, page } = session;
  const repeat = await page.evaluate(() => {
    const PM = (window as any).PM;
    let toggles = 0;
    const original = PM.toggle;
    PM.toggle = () => { toggles++; };
    const repeated = new KeyboardEvent('keydown', {
      key: ' ', code: 'Space', repeat: true, bubbles: true, cancelable: true,
    });
    window.dispatchEvent(repeated);
    const ordinary = new KeyboardEvent('keydown', {
      key: ' ', code: 'Space', bubbles: true, cancelable: true,
    });
    window.dispatchEvent(ordinary);
    PM.toggle = original;
    return {
      toggles,
      repeatedPrevented: repeated.defaultPrevented,
      ordinaryPrevented: ordinary.defaultPrevented,
    };
  });
  expect(repeat).toEqual({ toggles: 1, repeatedPrevented: false, ordinaryPrevented: true });

  const menuWired = await app.evaluate(({ Menu }) => {
    const split = Menu.getApplicationMenu()?.getMenuItemById('split');
    const fit = Menu.getApplicationMenu()?.getMenuItemById('fitComposition');
    const cut = Menu.getApplicationMenu()?.getMenuItemById('contextCut');
    const paste = Menu.getApplicationMenu()?.getMenuItemById('contextPaste');
    if (!split || !fit || !cut || !paste) return false;
    split.click();
    fit.click();
    cut.click();
    paste.click();
    return true;
  });
  expect(menuWired).toBe(true);
  await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(4);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Viewer.fit)).toBe(true);

  await page.evaluate(() => {
    const input = document.createElement('input');
    input.id = 'menu-context-field';
    input.value = 'select this text';
    document.body.append(input);
    input.focus();
    input.setSelectionRange(0, 0);
  });
  expect(await app.evaluate(({ Menu }) => {
    const selectAll = Menu.getApplicationMenu()?.getMenuItemById('contextSelectAll');
    if (!selectAll) return false;
    selectAll.click();
    return true;
  })).toBe(true);
  await expect.poll(() => page.evaluate(() => {
    const input = document.getElementById('menu-context-field') as HTMLInputElement | null;
    return input ? [input.selectionStart, input.selectionEnd] : null;
  })).toEqual([0, 16]);
  expect(await app.evaluate(({ Menu }) => {
    const cut = Menu.getApplicationMenu()?.getMenuItemById('contextCut');
    if (!cut) return false;
    cut.click();
    return true;
  })).toBe(true);
  await expect.poll(() => page.evaluate(() => {
    const input = document.getElementById('menu-context-field') as HTMLInputElement | null;
    return input?.value;
  })).toBe('');
  expect(await app.evaluate(({ Menu }) => {
    const paste = Menu.getApplicationMenu()?.getMenuItemById('contextPaste');
    if (!paste) return false;
    paste.click();
    return true;
  })).toBe(true);
  await expect.poll(() => page.evaluate(() => {
    const input = document.getElementById('menu-context-field') as HTMLInputElement | null;
    return input?.value;
  })).toBe('select this text');
  await page.evaluate(() => document.getElementById('menu-context-field')?.remove());
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('space toggles playback while the pointer is over the viewer but stays local in text fields', async ({ session }) => {
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause();
    PM.__spaceToggleCount = 0;
    PM.__originalSpaceToggle = PM.toggle;
    PM.toggle = () => { PM.__spaceToggleCount += 1; };
  });

  const viewer = page.locator('#viewer-stage, #stage').first();
  await viewer.hover({ position: { x: 40, y: 40 } });
  await page.keyboard.press('Space');
  expect(await page.evaluate(() => (window as any).PM.__spaceToggleCount)).toBe(1);

  await page.evaluate(() => {
    const input = document.createElement('input');
    input.id = 'space-shortcut-field';
    document.body.append(input);
    input.focus();
  });
  await page.keyboard.type(' ');
  expect(await page.evaluate(() => ({
    toggles: (window as any).PM.__spaceToggleCount,
    value: (document.getElementById('space-shortcut-field') as HTMLInputElement).value,
  }))).toEqual({ toggles: 1, value: ' ' });

  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.toggle = PM.__originalSpaceToggle;
    delete PM.__originalSpaceToggle;
    delete PM.__spaceToggleCount;
    document.getElementById('space-shortcut-field')?.remove();
  });
});

test('After Effects timeline keys navigate, retime layers, and move selected keyframes', async ({ session }) => {
  const { page } = session;
  await page.locator('body').click({ position: { x: 20, y: 100 } });
  const setup = await page.evaluate(() => {
    const PM = (window as any).PM;
    const layer = PM.firstSel();
    const timing = PM.Edit.apply({
      type: 'set_layer',
      target: layer.id,
      patch: { from: 2, duration: 4 },
    }, { label: 'Test layer timing', origin: 'test' });
    if (!timing.ok) throw new Error(timing.message);
    const key = PM.setKey(layer, 'opacity', 3, 50);
    PM.sel.keys = [key.i];
    PM.hist.clear();
    PM.setTime(0);
    return { layerId: layer.id, keyId: key.i };
  });

  await page.keyboard.press('i');
  await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBe(2);
  await page.keyboard.press('o');
  await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBe(6);

  await page.evaluate(() => (window as any).PM.setTime(5));
  await page.keyboard.press('[');
  await expect.poll(() => page.evaluate((layerId) => (window as any).PM.L(layerId)?.from, setup.layerId)).toBe(5);
  await page.keyboard.press('Meta+Z');
  await expect.poll(() => page.evaluate((layerId) => (window as any).PM.L(layerId)?.from, setup.layerId)).toBe(2);

  await page.evaluate(() => (window as any).PM.setTime(0));
  await page.keyboard.press('PageDown');
  await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBeCloseTo(1 / 30, 6);

  await page.keyboard.press('k');
  await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBe(2);

  const keyTimeBefore = await page.evaluate(({ layerId, keyId }) => {
    const PM = (window as any).PM;
    const layer = PM.L(layerId);
    PM.sel.keys = [keyId];
    return layer.p.opacity.kf.find((key: any) => key.i === keyId)?.t;
  }, setup);
  await page.keyboard.press('Alt+ArrowRight');
  await expect.poll(() => page.evaluate(({ layerId, keyId }) => {
    const layer = (window as any).PM.L(layerId);
    return layer.p.opacity.kf.find((key: any) => key.i === keyId)?.t;
  }, setup)).toBeCloseTo(keyTimeBefore + 1 / 30, 6);

  await page.keyboard.press('Meta+l');
  await expect.poll(() => page.evaluate((layerId) => (window as any).PM.L(layerId)?.lock, setup.layerId)).toBe(true);
  await page.keyboard.press('Meta+Shift+l');
  await expect.poll(() => page.evaluate((layerId) => (window as any).PM.L(layerId)?.lock, setup.layerId)).toBe(false);

  expect(session.diagnostics.pageErrors).toEqual([]);
});

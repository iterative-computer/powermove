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

test('professional shortcuts split, cut, paste, nudge, and respect focused fields', async ({ session }) => {
  const { app, page } = session;
  await page.locator('body').click({ position: { x: 20, y: 100 } });

  await app.evaluate(({ BrowserWindow }) => {
    const contents = BrowserWindow.getAllWindows()[0]?.webContents;
    if (!contents) throw new Error('Missing editor window');
    const modifiers: Array<'command' | 'control'> = process.platform === 'darwin' ? ['command'] : ['control'];
    contents.sendInputEvent({ type: 'keyDown', keyCode: 'b', modifiers });
    contents.sendInputEvent({ type: 'keyUp', keyCode: 'b', modifiers });
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
  await page.keyboard.press('Alt+ArrowRight');
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
    contents.sendInputEvent({ type: 'keyDown', keyCode: 'b', modifiers: [primary] });
    contents.sendInputEvent({ type: 'keyUp', keyCode: 'b', modifiers: [primary] });
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
    contents.sendInputEvent({ type: 'keyDown', keyCode: 'b', modifiers: [primary] });
    contents.sendInputEvent({ type: 'keyUp', keyCode: 'b', modifiers: [primary] });
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
  await expect.poll(() => page.evaluate(() => (window as any).PM.firstSel()?.on)).toBe(false);
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

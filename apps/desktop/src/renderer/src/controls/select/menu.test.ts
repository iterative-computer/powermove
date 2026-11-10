// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { closeSelectMenu, openSelectMenu } from './menu';

afterEach(()=>{closeSelectMenu();vi.runAllTimers();vi.useRealTimers();vi.unstubAllGlobals();document.body.replaceChildren();});

it('retires a closing listbox before another opens, keeping faded items out of focus and selection',()=>{
  vi.useFakeTimers();vi.stubGlobal('matchMedia',()=>({matches:false}));
  const anchor=document.createElement('button');document.body.append(anchor);
  const options=[{value:'frame',label:'Frame Selection'}],retiredPick=vi.fn(),activePick=vi.fn();
  const retired=openSelectMenu({anchor,options,value:'frame',onPick:retiredPick});
  retired.close();
  const active=openSelectMenu({anchor,options,value:'frame',onPick:activePick});
  expect(retired.el.isConnected).toBe(true); // The outgoing animation remains.
  expect(retired.el.inert).toBe(true);
  expect(retired.el.getAttribute('aria-hidden')).toBe('true');
  expect(document.querySelectorAll('[role="listbox"]:not([aria-hidden="true"])')).toHaveLength(1);
  retired.el.querySelector<HTMLElement>('[role="option"]')!.click();
  expect(retiredPick).not.toHaveBeenCalled();
  active.el.querySelector<HTMLElement>('[role="option"]')!.click();
  expect(activePick).toHaveBeenCalledExactlyOnceWith('frame');
  vi.advanceTimersByTime(200);expect(document.querySelectorAll('.pm-menu')).toHaveLength(0);
});

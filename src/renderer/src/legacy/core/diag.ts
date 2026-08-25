/* Ported from js/core/diag.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
const report = (tag: any, e: any) => {
    try {
      const msg = e && e.message ? e.message : String(e);
      const stack = e && e.stack ? String(e.stack).split('\n').slice(0, 3).join(' | ') : '';
      (window as any).webkit.messageHandlers.pmLog.postMessage('[js] ' + tag + ' ' + msg + ' @ ' + stack);
    } catch (_) { /* non-native contexts have no bridge */ }
  };

  window.addEventListener('error', ev => report('uncaught', ev.error || ev), true);
  window.addEventListener('unhandledrejection', ev => report('rejection', ev.reason || ev));

  ['setTimeout', 'setInterval'].forEach(name => {
    const orig = (window as any)[name];
    (window as any)[name] = function (fn: any, t: any) {
      if (typeof fn !== 'function') return orig.apply(window, arguments);
      const a = Array.prototype.slice.call(arguments, 2);
      const wrapped = function () { try { fn.apply(null, a); } catch (e) { report(name, e); throw e; } };
      return orig(wrapped, t);
    };
  });
  const raf = window.requestAnimationFrame;
  window.requestAnimationFrame = function (fn) {
    return raf.call(window, ts => { try { fn(ts); } catch (e) { report('rAF', e); throw e; } });
  };
}

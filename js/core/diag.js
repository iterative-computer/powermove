/* No longer loaded — superseded by src/renderer/src/legacy/core/diag.ts; kept for the legacy test oracle until Phase 6. */
/* Powermove — boot diagnostics. On file:// origins, uncaught errors arrive at
   window.onerror as cross-origin-masked "Script error." with no stack, which
   makes them undebuggable. Wrapping the async entry points (timers, rAF) lets
   us capture real messages + stacks and forward them to stderr via pmLog.
   The wrappers preserve timer ids, so clearTimeout/cancelAnimationFrame work. */
(() => {
  const report = (tag, e) => {
    try {
      const msg = e && e.message ? e.message : String(e);
      const stack = e && e.stack ? String(e.stack).split('\n').slice(0, 3).join(' | ') : '';
      window.webkit.messageHandlers.pmLog.postMessage('[js] ' + tag + ' ' + msg + ' @ ' + stack);
    } catch (_) { /* non-native contexts have no bridge */ }
  };

  window.addEventListener('error', ev => report('uncaught', ev.error || ev), true);
  window.addEventListener('unhandledrejection', ev => report('rejection', ev.reason || ev));

  ['setTimeout', 'setInterval'].forEach(name => {
    const orig = window[name];
    window[name] = function (fn, t) {
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
})();

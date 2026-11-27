// Driver-only startup/lifecycle progress. Never installed in the application.
// All virtual work and errors remain observable; nothing resets the clock.
export async function withBootClock(page, operation) {
  let pending = Promise.resolve(),
    busy = false,
    failure;
  const timer = setInterval(() => {
    if (busy) return;
    busy = true;
    pending = (async () => {
      for (const frame of page.frames()) {
        try {
          const error = await frame.evaluate(async () => {
            try {
              await globalThis.__extcheckClock?.advance(16);
              return null;
            } catch (error) {
              return String(error);
            }
          });
          failure ??= error;
        } catch {
          // Navigation can destroy a startup context. Readiness and the final
          // per-realm fixture check still have to succeed in the live document.
        }
      }
    })().finally(() => {
      busy = false;
    });
  }, 10);
  try {
    return await operation();
  } finally {
    clearInterval(timer);
    await pending;
    if (failure) throw Error(failure);
  }
}

/* Bundled PageApplier only. This document never evaluates extension modules. */
(() => {
  "use strict";
  const { base, generation } = PM_HOST_CONFIG;
  let panel = null,
    applier = null,
    after = 0,
    applied = 0,
    stopped = false;
  const waiting = [];
  const call = (value) => {
    const request = new XMLHttpRequest();
    request.open(
      "GET",
      base +
        "call?q=" +
        encodeURIComponent(JSON.stringify({ ...value, generation })),
      false,
    );
    request.send();
    const reply = JSON.parse(request.responseText);
    if (!reply.ok) throw Error(reply.error);
    return reply.value;
  };
  /** Frames whose DOM has applied. A canvas the document no longer has
   * releases its resource; it never stops the page. */
  const drawWaiting = async () => {
    while (waiting.length && waiting[0].dom_sequence <= applied) {
      const event = waiting.shift();
      const canvas = applier?.node(event.node);
      if (canvas?.localName === "canvas") {
        canvas.width = event.width;
        canvas.height = event.height;
        if (event.width && event.height) {
          const response = await fetch(base + "canvas?q=" + event.resource, { cache: "no-store" });
          if (response.ok) {
            const bitmap = await createImageBitmap(await response.blob());
            try { canvas.getContext("2d").drawImage(bitmap, 0, 0); }
            finally { bitmap.close(); }
          }
        }
      }
      call({ kind: "canvas_applied", panel, payload: {resource:event.resource} });
    }
  };
  const apply = async (event) => {
    if (event.kind === "create") {
      if (panel !== null)
        throw Error("Dedicated remote DOM page already owns a panel");
      panel = event.panel;
    } else {
      if (event.panel !== panel) throw Error("Unknown remote DOM panel");
      if (event.kind === "batch") {
        const batch = event.batch;
        if (batch.session.generation !== generation)
          throw Error("Stale remote DOM generation");
        if (!applier)
          applier = new PMRemoteDOM.PageApplier({
            document,
            session: batch.session,
            send: (payload) => {
              try {
                return call({ kind: "remote_dom", panel, payload });
              } catch (error) {
                stopped = true;
                applier?.dispose();
                try {
                  call({
                    kind: "failure",
                    panel: 0,
                    payload: "remote_dom_feedback_failed",
                  });
                } catch (_) {}
                throw error;
              }
            },
          });
        if (!applier.apply(batch)) throw Error("Remote DOM page fault");
        applied = batch.seq;
        await drawWaiting();
      } else if (event.kind === "canvas") {
        // A frame can overtake the DOM batch that creates its canvas; it waits.
        waiting.push(event);
        await drawWaiting();
      } else if (event.kind === "destroy") {
        waiting.length = 0;
        applier?.dispose();
        applier = null;
        panel = null;
      } else if (event.kind === "focus") {
        (event.node === null
          ? document.body
          : applier?.node(event.node)
        )?.focus();
      } else if (event.kind === "slot") {
        document.documentElement.hidden = !event.slot;
      } else throw Error("Unknown remote DOM envelope");
    }
  };
  async function poll() {
    try {
      while (!stopped) {
        const response = await fetch(base + "events?q=" + after, {
          cache: "no-store",
        });
        const message = await response.json();
        if (message.generation !== generation)
          throw Error("Stale host generation");
        for (const event of message.events) await apply(event);
        after = message.seq;
      }
    } catch (_) {
      stopped = true;
      try {
        call({ kind: "failure", panel: 0, payload: "remote_dom_stopped" });
      } catch (_) {}
    }
  }
  document.addEventListener(
    "DOMContentLoaded",
    () => {
      call({ kind: "ready", panel: 0, payload: { origin: location.origin } });
      void poll();
    },
    { once: true },
  );
})();

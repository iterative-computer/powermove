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
  const resourceImages = new WeakMap();
  // A removed image gives back its page URL (and the bytes behind it).
  const revoke = (node) => {
    const url = resourceImages.get(node);
    if (url) {
      URL.revokeObjectURL(url);
      resourceImages.delete(node);
    }
  };
  new MutationObserver((records) => {
    for (const record of records)
      for (const node of record.removedNodes) {
        if (node.nodeType !== 1 || node.isConnected) continue;
        revoke(node);
        for (const image of node.querySelectorAll?.("img") ?? []) revoke(image);
      }
  }).observe(document, { childList: true, subtree: true });
  for (const type of ["load", "error"])
    window.addEventListener(type, (event) => {
      if (resourceImages.has(event.target)) event.stopImmediatePropagation();
    }, true);
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
   * releases its resource quietly; a frame that cannot draw is reported. Neither
   * stops the page. */
  const drawWaiting = async () => {
    while (waiting.length && waiting[0].dom_sequence <= applied) {
      const event = waiting.shift();
      const node = applier?.node(event.node);
      let failed = !!node && node.localName !== "canvas" && node.localName !== "img";
      if (node?.localName === "img") {
        // The realm's image: its own bytes under a page-local URL. Its load is
        // the realm's, so the page's load and error never reach the extension.
        try {
          const response = await fetch(base + "canvas?q=" + event.resource, { cache: "no-store" });
          if (!response.ok) throw Error(String(response.status));
          const previous = resourceImages.get(node);
          const url = URL.createObjectURL(await response.blob());
          resourceImages.set(node, url);
          node.src = url;
          if (previous) URL.revokeObjectURL(previous);
        } catch (_) {
          failed = true;
        }
      } else if (node?.localName === "canvas") {
        node.width = event.width;
        node.height = event.height;
        if (event.width && event.height) {
          try {
            const response = await fetch(base + "canvas?q=" + event.resource, { cache: "no-store" });
            if (!response.ok) throw Error(String(response.status));
            const bitmap = await createImageBitmap(await response.blob());
            try { node.getContext("2d").drawImage(bitmap, 0, 0); }
            finally { bitmap.close(); }
          } catch (_) {
            failed = true;
          }
        }
      }
      call({ kind: "canvas_applied", panel, payload: {resource:event.resource, failed} });
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

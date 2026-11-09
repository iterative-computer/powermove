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
  // ui.modal: each [data-pm-modal] root shows in a sheet the host presents (a
  // child web view); the root moves into that document, styles and theme with
  // it, and the sheet closes when the extension's realm removes the root.
  const sheets = new Map();
  const present = (root) => {
    if (sheets.has(root)) return;
    const frame = root.getAttribute("data-pm-modal-frame") || "{}";
    const child = window.open("about:blank#pm-modal=" + encodeURIComponent(frame), "_blank");
    if (!child) return;
    sheets.set(root, child);
    const doc = child.document;
    doc.documentElement.setAttribute("data-pm-modal-doc", "");
    for (const attribute of document.documentElement.attributes)
      if (attribute.name !== "data-pm-modal-doc")
        doc.documentElement.setAttribute(attribute.name, attribute.value);
    for (const style of document.querySelectorAll("style"))
      doc.head.append(doc.importNode(style, true));
    doc.body.append(doc.adoptNode(root));
    // The sheet's input reaches the realm as the panel's own.
    applier?.attachWindow(child);
    doc.addEventListener("keydown", (event) => {
      if (event.key === "Escape") root.querySelector("[data-pm-modal-dismiss]")?.click();
    });
    new MutationObserver(() => {
      if (!root.isConnected || root.ownerDocument !== doc || root.parentNode !== doc.body)
        dismissSheet(root);
    }).observe(doc.body, { childList: true });
  };
  const dismissSheet = (root) => {
    const child = sheets.get(root);
    if (!child) return;
    sheets.delete(root);
    applier?.detachWindow(child);
    child.close();
  };
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
  // The engine's faces for the project's text, under the names its CSS uses,
  // so a hidden element an extension measures lays out as the engine draws.
  // Later batches wait for them: no read measures a fallback meanwhile.
  const loadFonts = async (faces) => {
    await Promise.all(faces.map(async (face) => {
      const font = new FontFace(face.family, `url("${base}font?q=${encodeURIComponent(face.key)}")`,
        { weight: String(face.weight), style: face.style });
      document.fonts.add(font);
      try { await font.load(); } catch (_) {}
    }));
    call({ kind: "fonts_loaded", panel: 0, payload: { keys: faces.map((face) => face.key) } });
  };
  const apply = async (event) => {
    if (event.kind === "fonts") {
      await loadFonts(event.faces);
    } else if (event.kind === "create") {
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
        for (const root of [...sheets.keys()]) dismissSheet(root);
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
      // ui.modal roots the realm appends to the body (it exists only now:
      // this script runs in the head).
      new MutationObserver((records) => {
        for (const record of records)
          for (const node of record.addedNodes)
            if (node.nodeType === 1 && node.matches?.("[data-pm-modal]")) present(node);
      }).observe(document.body, { childList: true });
      call({ kind: "ready", panel: 0, payload: { origin: location.origin } });
      void poll();
    },
    { once: true },
  );
})();

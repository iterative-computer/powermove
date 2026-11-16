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
  let nativeCSS = document.getElementById("pm-native")?.textContent || "";
  let nativeScheme = document.documentElement.dataset.pmScheme || "light";
  const nativeStyle = (doc) => {
    const root = doc.documentElement;
    root.dataset.pmScheme = nativeScheme;
    root.style.colorScheme = nativeScheme;
    root.dataset.theme = nativeScheme;
    let style = doc.getElementById("pm-native");
    if (!style) {
      style = doc.createElement("style");
      style.id = "pm-native";
      doc.head.prepend(style);
    }
    const mode = root.dataset.pmStyle || "full";
    const css =
      mode === "none"
        ? ""
        : mode === "tokens-only"
          ? nativeCSS.split("@layer pm-base")[0]
          : nativeCSS;
    if (style.textContent !== css) style.textContent = css;
  };
  const watchNativeStyle = (doc) => {
    new MutationObserver(() => nativeStyle(doc)).observe(doc.documentElement, {
      attributes: true,
      attributeFilter: ["data-pm-style"],
    });
    nativeStyle(doc);
  };
  watchNativeStyle(document);
  let scrolling;
  document.addEventListener(
    "scroll",
    () => {
      document.documentElement.classList.add("pm-scrolling");
      clearTimeout(scrolling);
      scrolling = setTimeout(
        () => document.documentElement.classList.remove("pm-scrolling"),
        700,
      );
    },
    true,
  );

  const present = (root) => {
    if (sheets.has(root)) return;
    const frame = root.getAttribute("data-pm-modal-frame") || "{}";
    const child = window.open(
      "about:blank#pm-modal=" + encodeURIComponent(frame),
      "_blank",
    );
    if (!child) return;
    sheets.set(root, child);
    const doc = child.document;
    doc.documentElement.setAttribute("data-pm-modal-doc", "");
    for (const attribute of document.documentElement.attributes)
      if (attribute.name !== "data-pm-modal-doc")
        doc.documentElement.setAttribute(attribute.name, attribute.value);
    for (const style of document.querySelectorAll("style"))
      doc.head.append(doc.importNode(style, true));
    watchNativeStyle(doc);
    doc.body.append(doc.adoptNode(root));
    // The sheet's input reaches the realm as the panel's own.
    applier?.attachWindow(child);
    doc.addEventListener("keydown", (event) => {
      if (event.key === "Escape")
        root.querySelector("[data-pm-modal-dismiss]")?.click();
    });
    new MutationObserver(() => {
      if (
        !root.isConnected ||
        root.ownerDocument !== doc ||
        root.parentNode !== doc.body
      )
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
    window.addEventListener(
      type,
      (event) => {
        if (resourceImages.has(event.target)) event.stopImmediatePropagation();
      },
      true,
    );
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
      let failed =
        !!node && node.localName !== "canvas" && node.localName !== "img";
      if (node?.localName === "img") {
        // The realm's image: its own bytes under a page-local URL. Its load is
        // the realm's, so the page's load and error never reach the extension.
        try {
          const response = await fetch(base + "canvas?q=" + event.resource, {
            cache: "no-store",
          });
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
            const response = await fetch(base + "canvas?q=" + event.resource, {
              cache: "no-store",
            });
            if (!response.ok) throw Error(String(response.status));
            const bitmap = await createImageBitmap(await response.blob());
            try {
              node.getContext("2d").drawImage(bitmap, 0, 0);
            } finally {
              bitmap.close();
            }
          } catch (_) {
            failed = true;
          }
        }
      }
      call({
        kind: "canvas_applied",
        panel,
        payload: { resource: event.resource, failed },
      });
    }
  };
  // The engine's faces for the project's text, under the names its CSS uses,
  // so a hidden element an extension measures lays out as the engine draws.
  // Later batches wait for them: no read measures a fallback meanwhile.
  const loadFonts = async (faces) => {
    await Promise.all(
      faces.map(async (face) => {
        const font = new FontFace(
          face.family,
          `url("${base}font?q=${encodeURIComponent(face.key)}")`,
          { weight: String(face.weight), style: face.style },
        );
        document.fonts.add(font);
        try {
          await font.load();
        } catch (_) {}
      }),
    );
    call({
      kind: "fonts_loaded",
      panel: 0,
      payload: { keys: faces.map((face) => face.key) },
    });
  };
  // Route each momentum/wheel event independently, as Electron's side docks do.
  document.addEventListener('wheel', (event) => {
    if (event.defaultPrevented || event.ctrlKey || event.metaKey || event.shiftKey || !event.deltaY) return;
    const root = event.target.closest?.('[data-pm-contribution]');
    if (!root) return;
    const delta = event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? root.clientHeight : 1);
    for (let node = event.target; node; node = node.parentElement) {
      const overflow = getComputedStyle(node).overflowY;
      const remaining = node.scrollHeight - node.clientHeight;
      if ((overflow === 'auto' || overflow === 'scroll') && remaining > 1 &&
          (delta < 0 ? node.scrollTop > 0.5 : node.scrollTop < remaining - 0.5)) return;
      if (node === root) break;
    }
    event.preventDefault();
    call({kind:'dock_scroll', panel:0, payload:{id:root.getAttribute('data-pm-contribution'),delta}});
  }, {passive:false});
  document.addEventListener('focusin', (event) => {
    const root = event.target.closest?.('[data-pm-contribution]');
    if (!root) return;
    const rect = event.target.getBoundingClientRect(), body = root.getBoundingClientRect();
    call({kind:'dock_reveal',panel:0,payload:{id:root.getAttribute('data-pm-contribution'),top:rect.top-body.top,height:rect.height}});
  });
  // Measure contribution content without running an idle measurement loop.
  const contributionHeight = (root) => {
    const origin = root.getBoundingClientRect().top;
    let height = root.scrollHeight;
    for (const node of root.querySelectorAll('*')) {
      const style = getComputedStyle(node);
      if (style.position === 'fixed' || !['auto', 'scroll'].includes(style.overflowY)) continue;
      height = Math.max(height, node.getBoundingClientRect().bottom - origin + Math.max(0, node.scrollHeight - node.clientHeight));
    }
    return height;
  };
  const reportedHeights = new WeakMap();
  let contentFrame = false;
  const measureContributions = () => {
    if (contentFrame) return;
    contentFrame = true;
    requestAnimationFrame(() => {
      contentFrame = false;
      for (const root of document.querySelectorAll('[data-pm-contribution]:not([data-pm-overscan])')) {
        const height = contributionHeight(root);
        if (height > 0 && Math.abs((reportedHeights.get(root) ?? -1) - height) > 1) {
          reportedHeights.set(root, height);
          call({kind:'dock_content',panel:0,payload:{id:root.getAttribute('data-pm-contribution'),height}});
        }
      }
    });
  };
  new MutationObserver(measureContributions).observe(document.body, {subtree:true,childList:true,attributes:true,characterData:true});
  document.addEventListener('load', measureContributions, true);
  document.fonts?.ready.then(measureContributions);
  measureContributions();
  // End contribution content measurement.
  const panelLayouts = new PMPanelLayout((frame) => requestAnimationFrame(frame), ({ id, rect }) => {
    const root = [...document.querySelectorAll('[data-pm-contribution]:not([data-pm-overscan])')]
      .find((node) => node.getAttribute('data-pm-contribution') === id);
    if (!root) return false;
    for (const [key, value] of Object.entries({position:'fixed', left:rect.x+'px', top:rect.y+'px', width:rect.w+'px', height:rect.h+'px', 'box-sizing':'border-box', 'min-width':'0', 'min-height':'0', 'max-width':rect.w+'px', 'max-height':rect.h+'px'}))
      root.style.setProperty(key, value, 'important');
  }, (placement) => {
    const root = [...document.querySelectorAll('[data-pm-contribution]:not([data-pm-overscan])')]
      .find((node) => node.getAttribute('data-pm-contribution') === placement.id);
    if (!root) return;
    const box = root.getBoundingClientRect();
    call({kind:'panel_layout_applied', panel:0, payload:{...placement,
      rect:{x:box.x,y:box.y,w:box.width,h:box.height}}});
  });
  document.addEventListener('scroll', (event) => {
    if (event.target.closest?.('[data-pm-overscan]')) return;
    const root = event.target.closest?.('[data-pm-contribution]');
    if (root) call({kind:'overscan_dirty',panel:0,payload:{id:root.getAttribute('data-pm-contribution')}});
  }, true);
  const apply = async (event) => {
    if (event.kind === "overscan_clear") {
      document.querySelector('[data-pm-overscan]')?.remove();
      requestAnimationFrame(() => requestAnimationFrame(() => call({kind:'overscan_cleared',panel:0,payload:{id:event.id,generation:event.generation}})));
    } else if (event.kind === "overscan_prepare") {
      document.querySelector('[data-pm-overscan]')?.remove();
      const root = [...document.querySelectorAll('[data-pm-contribution]:not([data-pm-overscan])')]
        .find((node) => node.getAttribute('data-pm-contribution') === event.id);
      if (!root) return;
      const clone = root.cloneNode(true);
      clone.setAttribute('data-pm-overscan', '');
      clone.setAttribute('aria-hidden', 'true');
      clone.setAttribute('inert', '');
      const rect = event.rect;
      clone.style.cssText = `position:fixed!important;left:${rect.x}px!important;top:0!important;width:${rect.w}px!important;height:${rect.h}px!important;max-height:${rect.h}px!important;box-sizing:border-box!important;overflow-x:hidden!important;overflow-y:${root.scrollHeight > root.clientHeight ? "scroll" : "hidden"}!important;pointer-events:none!important;z-index:2147483647`;
      document.body.append(clone);
      const originals = [root, ...root.querySelectorAll('*')];
      const copies = [clone, ...clone.querySelectorAll('*')];
      originals.forEach((node, i) => {
        if (node.scrollTop > 0) {
          const tail = document.createElement('div');
          tail.style.cssText = `height:${rect.h}px;flex:none;visibility:hidden;pointer-events:none`;
          copies[i].append(tail);
        }
        copies[i].scrollTop = node.scrollTop;
        copies[i].scrollLeft = node.scrollLeft;
        if (node.localName === 'canvas') copies[i].getContext('2d')?.drawImage(node, 0, 0);
      });
      requestAnimationFrame(() => requestAnimationFrame(() => call({kind:'overscan_ready',panel:0,payload:{id:event.id,generation:event.generation}})));
    } else if (event.kind === "panel_layout") {
      panelLayouts.enqueue(event.placements);
    } else if (event.kind === "fonts") {
      await loadFonts(event.faces);
    } else if (event.kind === "theme") {
      nativeCSS = event.css;
      nativeScheme = event.scheme;
      nativeStyle(document);
      for (const child of sheets.values()) nativeStyle(child.document);
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
        panelLayouts.refresh();
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
            if (node.nodeType === 1 && node.matches?.("[data-pm-modal]"))
              present(node);
      }).observe(document.body, { childList: true });
      call({ kind: "ready", panel: 0, payload: { origin: location.origin } });
      void poll();
    },
    { once: true },
  );
})();

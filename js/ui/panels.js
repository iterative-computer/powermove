/* No longer loaded — superseded by src/renderer/src/legacy/ui/panels.ts; kept for the legacy test oracle until Phase 6. */
/* Powermove — auxiliary panels: assets, shader editor, workspaces, takes, perf.
   The timeline owns layer ordering/visibility, so there is no separate Layers panel. */
(() => {
const PM = window.PM, h = PM.h;

/* ── Assets ────────────────────────────────────────────── */
function mediaDuration(seconds) {
  if (!(Number(seconds) > 0)) return '';
  const whole = Math.round(Number(seconds));
  const minutes = Math.floor(whole / 60);
  return minutes + ':' + String(whole % 60).padStart(2, '0');
}
function mediaSize(bytes) {
  const value = Number(bytes) || 0;
  if (!value) return '';
  if (value < 1024 * 1024) return Math.max(1, Math.round(value / 1024)) + ' KB';
  return (value / (1024 * 1024)).toFixed(value < 10 * 1024 * 1024 ? 1 : 0) + ' MB';
}
function mediaDetails(asset) {
  const parts = [];
  if (asset.kind !== 'audio' && asset.w && asset.h) parts.push(asset.w + '×' + asset.h);
  if (asset.dur) parts.push(mediaDuration(asset.dur));
  if (asset.size) parts.push(mediaSize(asset.size));
  return parts.join(' · ') || 'Ready to use';
}
function assetIcon(kind) { return kind === 'audio' ? 'clock' : kind === 'video' ? 'cam' : 'frame'; }

PM.registerPanel('assets', {
  title: 'Media', size: 200,
  build(body) {
    body.classList.add('assets-panel-body');
    const importButton = h('button.asset-import', { type: 'button', title: 'Import media (⌘I)', onclick: () => PM.pickFiles() }, PM.icon('plus'), h('span', 'Import'));
    const top = h('div.assets-top', importButton);
    const list = h('div.asset-list', { role: 'listbox', 'aria-label': 'Project media' });
    body.append(top, list);
    let selectedAssetId = null;
    const syncSelection = () => {
      list.querySelectorAll('.asset-card').forEach(row => {
        const selected = row.dataset.assetId === selectedAssetId;
        row.setAttribute('aria-selected', String(selected));
        row.tabIndex = selected ? 0 : -1;
      });
    };
    const selectAsset = (id, row) => {
      selectedAssetId = id;
      syncSelection();
      row && row.focus();
    };
    const deleteAsset = as => {
      const result = PM.hist.do('Delete media', () => PM.MediaImport.removeAsset(PM.proj, as.id));
      selectedAssetId = null;
      PM.sel.layers = PM.sel.layers.filter(id => !result.removedLayerIds.includes(id));
      PM.bus.emit('assets'); PM.bus.emit('layers'); PM.bus.emit('sel'); PM.bus.emit('project');
      PM.toast(result.removedLayers
        ? `Deleted ${as.name} and ${result.removedLayers} ${result.removedLayers === 1 ? 'layer' : 'layers'}`
        : `Deleted ${as.name}`);
    };
    const requestDelete = as => {
      const references = PM.MediaImport.referenceCount(PM.proj, as.id);
      if (!references) return deleteAsset(as);
      PM.modal({
        title: `Delete “${as.name}”?`, width: 420,
        body: h('div', { style: { color: 'var(--tx-2)', fontSize: '12.5px', lineHeight: 1.6 } },
          `This also removes ${references} ${references === 1 ? 'layer that uses' : 'layers that use'} this media. You can undo this.`),
        actions: [{ label: 'Cancel' }, { label: 'Delete', pri: true, run: () => deleteAsset(as) }],
      });
    };
    const paint = () => {
      list.textContent = '';
      const a = Object.values(PM.proj.assets);
      if (selectedAssetId && !PM.proj.assets[selectedAssetId]) selectedAssetId = null;
      a.forEach(as => {
        const live = PM.assets.get(as.id);
        const preview = h('span.asset-preview.' + as.kind, PM.icon(assetIcon(as.kind)));
        if (as.kind === 'image' && live && live.url) preview.appendChild(h('img', { src: live.url, alt: '' }));
        const add = h('button.asset-add', {
          type: 'button', title: 'Add to timeline', 'aria-label': 'Add ' + as.name + ' to timeline',
          onclick: event => { event.stopPropagation(); PM.cmd('addFromAsset', as.id); },
        }, PM.icon('plus'));
        const remove = h('button.asset-delete', {
          type: 'button', title: 'Delete media', 'aria-label': 'Delete ' + as.name,
          onclick: event => { event.stopPropagation(); selectAsset(as.id); requestDelete(as); },
        }, PM.icon('trash'));
        const actions = h('span.asset-actions', add, remove);
        const row = h('div.asset-card', {
          role: 'option', tabindex: selectedAssetId === as.id ? '0' : '-1', 'aria-selected': String(selectedAssetId === as.id),
          'data-asset-id': as.id, title: 'Select media · double-click to add to the timeline',
          onclick: event => { if (!event.target.closest('button')) selectAsset(as.id, row); },
          ondblclick: event => { if (!event.target.closest('button')) PM.cmd('addFromAsset', as.id); },
          onkeydown: event => {
            if (event.key === 'Delete' || event.key === 'Backspace') { event.preventDefault(); event.stopPropagation(); requestDelete(as); }
            else if (event.key === 'Enter') { event.preventDefault(); PM.cmd('addFromAsset', as.id); }
            else if (event.key === ' ') { event.preventDefault(); selectAsset(as.id, row); }
          },
        }, preview, h('span.asset-copy', h('b', as.name), h('small', mediaDetails(as))), actions);
        list.appendChild(row);
      });
      if (!a.length) list.appendChild(h('div.asset-empty', PM.icon('project'), h('b', 'No imported media'), h('span', 'Images, video, and audio stay with this project.')));
    };
    paint(); PM.bus.on('assets', paint);
  },
});

/* ── Effects browser ───────────────────────────────────── */
PM.registerPanel('fxbrowser', {
  title: 'Effects', size: 220,
  build(body) {
    const wrap = h('div', { style: { padding: '6px' } });
    const groups = {};
    Object.entries(PM.FX).forEach(([k, d]) => (groups[d.group] = groups[d.group] || []).push([k, d]));
    Object.entries(groups).forEach(([g, list]) => {
      wrap.appendChild(h('div.sec', { style: { margin: '8px 6px 4px' } }, g));
      list.forEach(([k, d]) => {
        const row = h('div.lyr', h('span.nm', d.label), h('span.idx', '+'));
        row.onclick = (event) => {
          /* A native double-click emits click, click, then dblclick. The old
             shared click/dblclick handler therefore added three effects, so
             one Undo appeared to do nothing. Treat the first click as the one
             activation and ignore the follow-up click. */
          if (event.detail > 1) return;
          const L = PM.firstSel();
          if (!L) return PM.toast('Select a layer first');
          PM.Edit.apply({ type: 'add_effect', target: L.id, effect: k }, { label: 'Add ' + d.label, origin: 'effects-panel' });
          PM.Inspector.refresh(); PM.invalidate();
        };
        wrap.appendChild(row);
      });
    });
    body.appendChild(wrap);
  },
});

/* ── Shader editor ─────────────────────────────────────── */
PM.registerPanel('shader', {
  title: 'Shader', noscroll: true, size: 320,
  build(body, inst) {
    const ta = h('textarea.code', { spellcheck: 'false' });
    const bar = h('div.codebar');
    const status = h('span', 'ready');
    const presets = h('button.chip', { style: { height: '20px' } }, 'Presets');
    presets.onpointerdown = (e) => {
      e.preventDefault();
      PM.menu(presets, Object.keys(PM.SHADER_PRESETS).map(n => ({
        label: n, run: () => { const L = target(); if (!L) return; ta.value = PM.SHADER_PRESETS[n]; apply('Shader preset'); },
      })));
    };
    bar.append(status, h('span', { style: { flex: 1 } }), presets,
      h('button.chip', { style: { height: '20px' }, onclick: () => apply() }, 'Compile ⌘↵'));
    body.append(ta, bar);
    inst.ta = ta;

    const target = () => { const L = PM.firstSel(); return L && L.type === 'shader' ? L : PM.proj.layers.find(l => l.type === 'shader'); };
    const load = () => {
      const L = target();
      if (!L) { ta.value = '// Select or create a shader layer (⌘⇧G)'; ta.disabled = true; status.textContent = 'no shader layer'; return; }
      ta.disabled = false;
      if (document.activeElement !== ta) ta.value = L.d.code;
      showErr(L);
    };
    function showErr(L) {
      const meta = PM.UIState.getShaderMeta(L);
      const err = PM.GL.compileError(meta.shaderKey);
      status.className = err ? 'bad' : 'ok';
      status.textContent = err ? err.split('\n')[0].slice(0, 90) : '✓ compiled · ' + meta.udefs.length + ' uniforms';
    }
    function apply(label = 'Edit shader') {
      const L = target(); if (!L) return;
      PM.Edit.apply({ type: 'set_content', target: L.id, patch: { code: ta.value } }, { label, origin: 'shader-panel' });
      PM.syncShaderUniforms(L);
      PM.GL.dropProgram(PM.UIState.getShaderMeta(L).shaderKey);
      PM.invalidate();
      requestAnimationFrame(() => { showErr(L); PM.Inspector.refresh(); });
    }
    let tmr = 0;
    ta.addEventListener('input', () => { clearTimeout(tmr); tmr = setTimeout(apply, 420); });
    ta.addEventListener('keydown', (e) => {
      e.stopPropagation();
      if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); apply(); }
      if (e.key === 'Tab') {
        e.preventDefault();
        const s = ta.selectionStart;
        ta.value = ta.value.slice(0, s) + '  ' + ta.value.slice(ta.selectionEnd);
        ta.selectionStart = ta.selectionEnd = s + 2;
      }
    });
    load();
    PM.bus.on('sel', load); PM.bus.on('layers', load);
  },
});

PM.openShaderEditor = (L) => {
  if (L) PM.selectLayers(L.id);
  if (!PM.Layout.hasPanel(PM.WS.current, 'shader')) {
    PM.WS.mutate(w => PM.Layout.addPanel(w, 'shader', 'center'));
  }
  PM.Layout.refresh('shader');
};

/* ── Workspaces ────────────────────────────────────────── */
PM.registerPanel('workspaces', {
  title: 'Workspaces', size: 200,
  build(body) {
    const wrap = h('div', { style: { padding: '6px' } });
    body.appendChild(wrap);
    const paint = () => {
      wrap.textContent = '';
      PM.WS.list().forEach(w => {
        const row = h('div.lyr' + (w.id === PM.WS.current.id ? '.sel' : ''),
          h('span.sw2', { style: { background: (w.theme && w.theme.accent) || 'var(--accent)' } }),
          h('span.nm', w.name),
          w.builtin ? null : h('button.stopwatch', { onclick: (e) => { e.stopPropagation(); PM.WS.remove(w.id); } }, PM.icon('x')));
        row.onclick = () => PM.WS.activate(w.id);
        wrap.appendChild(row);
      });
      wrap.appendChild(h('div', { style: { display: 'flex', gap: '6px', padding: '8px 4px' } },
        h('button.chip', { onclick: () => PM.WS.saveAsNew() }, PM.icon('plus'), 'Save current'),
        h('button.chip', { onclick: () => PM.WS.editJSON() }, PM.icon('code'), 'JSON')));
      wrap.appendChild(h('div.empty', { style: { padding: '10px 8px', textAlign: 'left' } },
        'Shake the pointer, then drag across a panel to redesign or add an interface section.'));
    };
    paint(); PM.bus.on('workspaces', paint); PM.bus.on('layout', paint);
  },
});

/* ── Takes ─────────────────────────────────────────────── */
PM.registerPanel('takes', {
  title: 'Takes', size: 180,
  build(body) {
    const wrap = h('div', { style: { padding: '6px' } });
    body.appendChild(wrap);
    const paint = () => {
      wrap.textContent = '';
      const list = PM.takes.all();
      list.forEach(t => {
        const row = h('div.lyr', h('span.nm', t.label),
          h('span.idx', new Date(t.at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })),
          h('button.stopwatch', { onclick: (e) => { e.stopPropagation(); PM.takes.drop(t.id); } }, PM.icon('x')));
        row.onclick = () => PM.takes.restore(t.id);
        wrap.appendChild(row);
      });
      if (!list.length) wrap.appendChild(h('div.empty', 'Save a take before exploring a new motion direction.'));
      wrap.appendChild(h('button.chip', { style: { margin: '6px' }, onclick: () => PM.takes.save() }, PM.icon('plus'), 'Save take'));
    };
    paint(); PM.bus.on('takes', paint);
  },
});

/* ── Performance ───────────────────────────────────────── */
PM.registerPanel('perf', {
  title: 'Performance', size: 150,
  build(body) {
    const wrap = h('div.insp');
    body.appendChild(wrap);
    const rows = {};
    const add = (k) => { const v = h('span', { style: { fontSize: '11.5px', fontVariantNumeric: 'tabular-nums' } }, '—'); rows[k] = v; wrap.appendChild(PM.row(k, v)); };
    ['FPS', 'Frame ms', 'GL draws', 'FX passes', 'Programs', 'Raster cache', 'Layers', 'Keyframes'].forEach(add);
    const sync = () => {
      const s = PM.GL.stats;
      rows['FPS'].textContent = PM.perf.fps;
      rows['Frame ms'].textContent = PM.round(PM.perf.ms, 2);
      rows['GL draws'].textContent = s.draws;
      rows['FX passes'].textContent = s.passes;
      rows['Programs'].textContent = s.progs;
      rows['Raster cache'].textContent = PM.rasterStats().size;
      rows['Layers'].textContent = PM.proj.layers.length;
      rows['Keyframes'].textContent = PM.proj.layers.reduce((n, L) => n + PM.allProps(L).reduce((m, p) => m + p.prop.kf.length, 0), 0);
    };
    sync(); PM.bus.on('draw:status', sync);
  },
});

/* ── Notes ─────────────────────────────────────────────── */
PM.registerPanel('notes', {
  title: 'Notes', size: 180,
  build(body) {
    const ta = h('textarea', {
      style: { width: '100%', height: '100%', minHeight: '120px', background: 'transparent', padding: '10px 12px', fontSize: '12.5px', lineHeight: 1.6, resize: 'none', color: 'var(--tx-2)' },
      placeholder: 'Direction notes — saved with this project as its creative brief.',
    }, PM.proj.notes || '');
    ta.addEventListener('input', () => { PM.proj.notes = ta.value; });
    ta.addEventListener('keydown', e => e.stopPropagation());
    body.appendChild(ta);
  },
});

})();

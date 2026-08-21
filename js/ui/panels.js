/* Powermove — auxiliary panels: layers, assets, shader editor, workspaces, takes, perf. */
(() => {
const PM = window.PM, h = PM.h;

/* ── Layers ────────────────────────────────────────────── */
PM.registerPanel('layers', {
  title: 'Layers', size: 220,
  build(body) {
    const list = h('div', { style: { padding: '6px' } });
    body.appendChild(list);
    const paint = () => {
      list.textContent = '';
      PM.proj.layers.forEach((L, i) => {
        const row = h('div.lyr' + (PM.sel.layers.includes(L.id) ? '.sel' : ''),
          h('span.idx', String(i + 1).padStart(2, '0')),
          h('span.sw2', { style: { background: L.color } }),
          h('span.nm', L.name),
          h('button.stopwatch' + (L.on ? '.on' : ''), {
            onclick: (e) => { e.stopPropagation(); PM.Edit.apply({ type: 'set_layer', target: L.id, patch: { visible: !L.on } }, { label: 'Visibility', origin: 'layers-panel' }); PM.invalidate(); },
          }, PM.icon(L.on ? 'eye' : 'eyeoff')));
        row.onclick = (e) => PM.selectLayers(L.id, e.shiftKey || e.metaKey);
        list.appendChild(row);
      });
      if (!PM.proj.layers.length) list.appendChild(h('div.empty', 'No layers yet.\nPress ⌘T for text, ⌘Y for a solid,\nor ask the assistant.'));
    };
    paint();
    PM.bus.on('layers', paint); PM.bus.on('sel', paint);
  },
});

/* ── Assets ────────────────────────────────────────────── */
PM.registerPanel('assets', {
  title: 'Project', size: 200,
  build(body) {
    const list = h('div', { style: { padding: '6px' } });
    const drop = h('div', {
      style: { margin: '6px', padding: '14px', border: '1px dashed var(--line-2)', borderRadius: '10px', textAlign: 'center', color: 'var(--tx-3)', fontSize: '11.5px' },
    }, 'Drop media here');
    body.append(list, drop);
    const paint = () => {
      list.textContent = '';
      const a = Object.values(PM.proj.assets);
      a.forEach(as => {
        const row = h('div.lyr', h('span.sw2', { style: { background: as.kind === 'audio' ? 'var(--blue)' : as.kind === 'video' ? 'var(--blue-deep)' : 'var(--gray)' } }),
          h('span.nm', as.name), h('span.idx', as.kind[0].toUpperCase()));
        row.ondblclick = () => PM.cmd('addFromAsset', as.id);
        list.appendChild(row);
      });
      if (!a.length) list.appendChild(h('div.empty', 'No media imported.'));
    };
    paint(); PM.bus.on('assets', paint);
    ['dragover', 'drop'].forEach(ev => drop.addEventListener(ev, (e) => {
      e.preventDefault();
      if (ev === 'drop') PM.importFiles([...e.dataTransfer.files]);
    }));
    drop.onclick = () => PM.pickFiles();
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
        row.ondblclick = row.onclick = () => {
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
      const err = PM.GL.compileError(L._shaderKey);
      status.className = err ? 'bad' : 'ok';
      status.textContent = err ? err.split('\n')[0].slice(0, 90) : '✓ compiled · ' + (L._udefs || []).length + ' uniforms';
    }
    function apply(label = 'Edit shader') {
      const L = target(); if (!L) return;
      PM.Edit.apply({ type: 'set_content', target: L.id, patch: { code: ta.value } }, { label, origin: 'shader-panel' });
      PM.syncShaderUniforms(L);
      PM.GL.dropProgram(L._shaderKey);
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
        'Ask the assistant: “make a workspace for shader work”, “hide the timeline”, “put properties on the left”.'));
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
      if (!list.length) wrap.appendChild(h('div.empty', 'Takes are saved automatically\nbefore the assistant edits.'));
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
    const add = (k) => { const v = h('span', { style: { fontFamily: 'var(--f-mono)', fontSize: '11.5px' } }, '—'); rows[k] = v; wrap.appendChild(PM.row(k, v)); };
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
      placeholder: 'Direction notes — the assistant reads these as creative brief.',
    }, PM.proj.notes || '');
    ta.addEventListener('input', () => { PM.proj.notes = ta.value; });
    ta.addEventListener('keydown', e => e.stopPropagation());
    body.appendChild(ta);
  },
});
})();

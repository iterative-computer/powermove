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
            onclick: (e) => { e.stopPropagation(); PM.hist.do('Visibility', () => { L.on = !L.on; }); PM.invalidate(); },
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

/* ── Media browser ─────────────────────────────────────── */
PM.registerPanel('assets', {
  title: 'Media', size: 200, noscroll: true,
  header(hdr) {
    if (hdr.querySelector('.mb-add')) return;
    hdr.appendChild(h('button.iconbtn.mb-add', {
      title: 'Import media (⌘I)',
      onclick: () => PM.pickFiles({ place: false }),
    }, PM.icon('plus')));
  },
  build(body) {
    const st = { q: '', kind: 'all', list: false, sel: null };
    const bar = h('div.mb-bar');
    const q = h('input.mb-q', { type: 'search', placeholder: 'Search media', spellcheck: 'false' });
    q.addEventListener('input', () => { st.q = q.value; paint(); });
    q.addEventListener('keydown', e => e.stopPropagation());
    const kinds = h('div.mb-kinds');
    ['all', 'image', 'video', 'audio'].forEach(k => {
      const b = h('button.mb-f' + (k === 'all' ? '.on' : ''), k === 'all' ? 'All' : k[0].toUpperCase() + k.slice(1));
      b.onclick = () => { st.kind = k; [...kinds.children].forEach(c => c.classList.toggle('on', c === b)); paint(); };
      kinds.appendChild(b);
    });
    const view = h('button.iconbtn', { title: 'List view' }, PM.icon('layers'));
    view.onclick = () => {
      st.list = !st.list;
      view.title = st.list ? 'Grid view' : 'List view';
      view.textContent = ''; view.appendChild(PM.icon(st.list ? 'grid' : 'layers'));
      paint();
    };
    bar.append(q, h('div.mb-row', kinds, view));
    const grid = h('div.mb-grid');
    body.append(bar, grid);
    body.classList.add('mb');

    const fmtDur = (s) => {
      if (!s || !isFinite(s)) return '';
      const m = Math.floor(s / 60), r = Math.round(s % 60);
      return m + ':' + String(r).padStart(2, '0');
    };
    const metaOf = (as) => {
      if (as.kind === 'audio') return fmtDur(as.dur) || 'Audio';
      if (as.kind === 'video') return [fmtDur(as.dur), as.w ? as.w + '×' + as.h : ''].filter(Boolean).join(' · ');
      return as.w ? as.w + '×' + as.h : 'Image';
    };
    const shotOf = (as) => {
      const live = PM.assets.get(as.id);
      const shot = h('div.mb-shot');
      if (!live) { shot.appendChild(h('div.mb-ph', PM.icon(as.kind === 'audio' ? 'audio' : 'cam'))); shot.classList.add('off'); return shot; }
      if (as.kind === 'audio') { shot.appendChild(h('div.mb-ph', PM.icon('audio'))); return shot; }
      if (live.thumbEl) { shot.appendChild(live.thumbEl); return shot; }
      if (as.kind === 'image' && live.el) {
        const img = h('img');
        img.src = live.url || (live.el.toDataURL && live.el.toDataURL()) || live.el.src;
        live.thumbEl = img;
        shot.appendChild(img);
        return shot;
      }
      if (as.kind === 'video' && live.el) {
        const img = h('img');
        const v = live.el;
        const cap = () => {
          try {
            const c = document.createElement('canvas');
            c.width = Math.max(2, v.videoWidth || 320);
            c.height = Math.max(2, v.videoHeight || 180);
            c.getContext('2d').drawImage(v, 0, 0, c.width, c.height);
            img.src = c.toDataURL('image/jpeg', .7);
          } catch (e) { }
        };
        if (v.readyState >= 2) cap();
        else v.addEventListener('seeked', cap, { once: true });
        try { v.currentTime = Math.min(.12, (v.duration || 1) * .04); } catch (e) { }
        live.thumbEl = img;
        shot.appendChild(img);
        return shot;
      }
      shot.appendChild(h('div.mb-ph', PM.icon('cam')));
      return shot;
    };

    const paint = () => {
      grid.textContent = '';
      grid.classList.toggle('list', st.list);
      const qv = st.q.trim().toLowerCase();
      const items = Object.values(PM.proj.assets).filter(as => {
        if (st.kind !== 'all' && as.kind !== st.kind) return false;
        if (qv && !as.name.toLowerCase().includes(qv)) return false;
        return true;
      });
      if (!items.length) {
        const empty = Object.values(PM.proj.assets).length;
        grid.appendChild(h('div.mb-empty',
          empty ? 'Nothing matches.' : 'Drop footage, stills, or audio',
          empty ? null : h('span', 'or click to import · ⌘I')));
        return;
      }
      items.forEach(as => {
        const card = h('div.mb-card' + (st.sel === as.id ? '.on' : ''),
          { draggable: 'true' },
          shotOf(as),
          h('div.mb-cap',
            h('div.mb-name', { title: as.name }, as.name.replace(/\.[^.]+$/, '')),
            h('div.mb-meta', metaOf(as))));
        card.onclick = (e) => { st.sel = as.id; paint(); };
        card.ondblclick = () => PM.cmd('addFromAsset', as.id);
        card.oncontextmenu = (e) => {
          e.preventDefault();
          st.sel = as.id; paint();
          PM.menu(document.body, [
            { header: as.name },
            { label: 'Add to composition', run: () => PM.cmd('addFromAsset', as.id) },
            '-',
            { label: 'Delete from media', run: () => PM.hist.do('Remove media', () => PM.assets.remove(as.id)) },
          ], { x: e.clientX, y: e.clientY });
        };
        card.addEventListener('dragstart', (e) => {
          e.dataTransfer.setData('application/x-pm-asset', as.id);
          e.dataTransfer.setData('text/plain', as.id);
          e.dataTransfer.effectAllowed = 'copy';
        });
        grid.appendChild(card);
      });
    };
    paint();
    PM.bus.on('assets', paint);

    const hot = (on) => body.classList.toggle('hot', on);
    body.addEventListener('dragover', (e) => {
      if (![...e.dataTransfer.types].includes('Files')) return;
      e.preventDefault(); e.stopPropagation(); hot(true);
    });
    body.addEventListener('dragleave', (e) => { if (!body.contains(e.relatedTarget)) hot(false); });
    body.addEventListener('drop', (e) => {
      if (![...e.dataTransfer.types].includes('Files')) return;
      e.preventDefault(); e.stopPropagation(); hot(false);
      PM.importFiles([...e.dataTransfer.files], { place: false });
    });
    grid.addEventListener('click', (e) => {
      if (Object.values(PM.proj.assets).length) return;
      if (e.target === grid || e.target.closest('.mb-empty')) PM.pickFiles({ place: false });
    });
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
          PM.hist.do('Add ' + d.label, () => { const fx = PM.mkEffect(k); fx.open = true; L.fx.push(fx); });
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
        label: n, run: () => { const L = target(); if (!L) return; PM.hist.do('Shader preset', () => { L.d.code = PM.SHADER_PRESETS[n]; }); ta.value = L.d.code; apply(); },
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
    function apply() {
      const L = target(); if (!L) return;
      PM.hist.do('Edit shader', () => { L.d.code = ta.value; });
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

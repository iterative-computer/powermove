/* Powermove — auxiliary panels: assets, shader editor, workspaces, takes, perf.
   The timeline owns layer ordering/visibility, so there is no separate Layers panel. */
(() => {
const PM = window.PM, h = PM.h;

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

/* ── Generative library ────────────────────────────────── */
/* Browsable home for AI-generated and hand-saved sections (layer groups) and
   looks (shader presets). Everything carries a live thumbnail captured when
   it was saved. */
PM.registerPanel('library', {
  title: 'Generative', size: 250,
  build(body) {
    const wrap = h('div', { style: { padding: '6px' } });
    const bar = h('div', { style: { display: 'flex', gap: '6px', padding: '2px' } },
      h('button.chip', {
        title: 'Save selected layers as a reusable section',
        onclick: () => { const sels = PM.selLayers(); PM.Library.saveSection(null, sels.length ? sels.map(l => l.id) : null); paint(); },
      }, PM.icon('plus'), 'Section'),
      h('button.chip', {
        title: 'Save the selected shader layer as a look',
        onclick: () => { PM.Library.saveLook(); paint(); },
      }, PM.icon('wand'), 'Look'));
    body.append(bar, wrap);

    const card = (entry, kind) => {
      const img = h('img', { src: entry.thumb || '', alt: entry.name });
      const cv = h('div.lib-thumb', img);
      if (!entry.thumb) cv.appendChild(h('div.lib-empty', kind === 'sections' ? '▣' : '✦'));
      const c = h('div.lib-card',
        cv,
        h('div.lib-name', entry.name),
        h('div.lib-acts',
          h('button.stopwatch', { title: kind === 'sections' ? 'Insert at playhead' : 'Add shader layer', onclick: (e) => { e.stopPropagation(); kind === 'sections' ? PM.Library.insertSection(entry.id) : PM.Library.applyLook(entry.id); } }, PM.icon('plus')),
          h('button.stopwatch', { title: 'Details, notes & refinement', onclick: (e) => { e.stopPropagation(); detail(entry, kind, paint); } }, PM.icon('grip')),
          h('button.stopwatch', { title: 'Remove from library', onclick: (e) => { e.stopPropagation(); PM.Library.drop(kind, entry.id); paint(); } }, PM.icon('x'))));
      c.onclick = () => detail(entry, kind, paint);
      return c;
    };

    /* Detail studio: big preview, layer contents, comment thread, and a
       one-click refine prompt handed to the assistant. */
    function detail(entry, kind, refresh) {
      const isSection = kind === 'sections';
      const cmts = h('div.lib-comments');
      const paintCmts = () => {
        cmts.textContent = '';
        (entry.comments || []).forEach(c => cmts.appendChild(
          h('div.lib-cmt',
            h('span.cmt-at', new Date(c.at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })),
            h('div.cmt-tx', c.text))));
        if (!(entry.comments || []).length) cmts.appendChild(h('div.empty', 'No notes yet — leave direction for future you.'));
      };
      const noteInp = h('textarea', { placeholder: 'Add a comment — creative intent, tweaks to try…', rows: 2 });
      noteInp.addEventListener('keydown', e => e.stopPropagation());
      const addNote = () => {
        if (!noteInp.value.trim()) return;
        PM.Library.comment(kind, entry.id, noteInp.value);
        noteInp.value = '';
        paintCmts();
      };

      const refine = () => {
        if (isSection) PM.Library.insertSection(entry.id);
        else PM.Library.applyLook(entry.id);
        PM.ChatRail.open(true);
        const ci = PM.$('#cinput');
        if (ci) {
          ci.value = (isSection
            ? `Refine the inserted section “${entry.name}” (its layers are selected). `
            : `Refine the applied look “${entry.name}” (its shader layer is selected). `)
            + ((entry.comments || []).slice(-1)[0] ? `Latest note: “${entry.comments.slice(-1)[0].text}”. ` : '');
          ci.dispatchEvent(new Event('input'));
        }
      };

      const layersInfo = isSection
        ? h('div.lib-layers', ...(entry.layers || []).map(l =>
            h('span.lyr-tag', PM.TYPE_META[l.type] ? PM.TYPE_META[l.type].label : l.type, ': ', l.name)))
        : null;

      const body = h('div.lib-detail',
        h('div.lib-bigthumb', h('img', { src: entry.thumb || '', alt: entry.name })),
        layersInfo,
        h('div.sec', { style: { margin: '10px 2px 6px' } }, 'Notes'),
        cmts,
        h('div.field', noteInp),
        h('button.chip', { style: { marginTop: '-4px' }, onclick: addNote }, 'Add note'));

      PM.modal({
        title: entry.name,
        body, width: 560,
        actions: [
          { label: 'Delete', run: () => { PM.Library.drop(kind, entry.id); refresh && refresh(); } },
          { label: 'Refine with assistant', pri: true, run: refine },
          { label: isSection ? 'Insert at playhead' : 'Apply look', run: () => {
              isSection ? PM.Library.insertSection(entry.id) : PM.Library.applyLook(entry.id);
              refresh && refresh();
            } },
        ],
      });
      paintCmts();
    }

    function paint() {
      wrap.textContent = '';
      const L = PM.Library.all();
      if (L.sections.length) {
        wrap.appendChild(h('div.sec', { style: { margin: '8px 4px 4px' } }, 'Sections'));
        wrap.appendChild(h('div.lib-grid', ...L.sections.map(s => card(s, 'sections'))));
      }
      if (L.looks.length) {
        wrap.appendChild(h('div.sec', { style: { margin: '10px 4px 4px' } }, 'Looks · shaders'));
        wrap.appendChild(h('div.lib-grid', ...L.looks.map(k => card(k, 'looks'))));
      }
      if (!L.sections.length && !L.looks.length) {
        wrap.appendChild(h('div.empty', { style: { textAlign: 'left', padding: '10px 6px' } },
          'Nothing saved yet.\n\nSelect layers and press Section to keep a reusable piece of the composition — or ask the assistant to “save this as a section”. Generated shaders can be kept as Looks.'));
      }
    }
    paint();
    PM.bus.on('library', paint);
    PM.bus.on('layers', paint);
  },
});
})();

/* Ported from js/ui/projects.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import { subscribeForkUpdates, updateAll, type ForkUpdate } from '../../shell/fork-updates';
import { installUpdate, subscribeAppUpdates } from '../../shell/app-updates';
import type { AppUpdateState } from '../../../../shared/ipc';
import { mountNavGlide } from '../../controls/nav-glide';
import { mount, unmount } from 'svelte';
import AccountButton from '../../cloud/AccountButton.svelte';

export function install(PM: PMRegistry): void {
const h = PM.h;
const PAGE_SIZE = 60;
const wasOpen = !!PM.ProjectsScreen?.isOpen;
const previousSection = PM.ProjectsScreen?.section;
PM.__disposeProjectsScreen?.();
const S: any = {
  el: null, nav: null, grid: null, title: null, count: null, search: null,
  section: PM.store.get('projectsSection', 'recents'),
  view: PM.store.get('projectsView', 'grid'),
  sort: PM.store.get('projectsSort', 'recent'),
  page: 0, queryKey: '',
};
if (!['recents', 'projects', 'trash'].includes(S.section)) S.section = 'recents';

PM.ProjectsScreen = {
  get isOpen() { return !!S.el && S.el.classList.contains('on'); },
  get section() { return S.section; },
  show(section: any) {
    ensure();
    if (section && ['recents', 'projects', 'trash'].includes(section)) S.section = section;
    paint(); S.el.classList.add('on'); PM.bus.emit('projects:screen');
  },
  hide() {
    // Nothing to fall back to: this window has no composition behind the screen.
    if (!PM.proj?.id || PM.isHomeProject?.()) return;
    if (S.el) S.el.classList.remove('on');
    PM.bus.emit('projects:screen');
  },
  toggle() { this.isOpen ? this.hide() : this.show(); },
};

function ensure() {
  if (S.el) return;
  S.search = h('input', { type: 'search', placeholder: 'Search projects', 'aria-label': 'Search projects' });
  S.search.addEventListener('input', paint);
  S.nav = h('div.ps-nav');
  /* Forked built-ins that fell behind the shipped version. The agent does the
     merge; this block only says how many and offers the one action. */
  S.updates = h('div.ps-updates', { hidden: true });
  const paintUpdates = (pending: readonly ForkUpdate[]) => {
    S.updates.textContent = '';
    S.updates.hidden = pending.length === 0;
    if (!pending.length) return;
    const names = pending.map((update) => update.name).join(', ');
    S.updates.append(
      h('b', pending.length === 1 ? '1 extension needs updating' : `${pending.length} extensions need updating`),
      h('span', `${names} ${pending.length === 1 ? 'was' : 'were'} forked from an older built-in. Your agent can merge the new version while keeping your changes.`),
      h('button.btn', { onclick: () => { updateAll(); PM.ProjectsScreen.hide(); } }, 'Update with agent'));
  };
  S.offUpdates = subscribeForkUpdates(paintUpdates);
  /* A new app version staged by the background updater. Installing is a
     normal quit (session saved) followed by a relaunch. */
  S.appUpdate = h('div.ps-updates', { hidden: true });
  const paintAppUpdate = (state: AppUpdateState | null) => {
    S.appUpdate.textContent = '';
    S.appUpdate.hidden = state?.status !== 'ready';
    if (state?.status !== 'ready') return;
    S.appUpdate.append(
      h('b', state.version ? `Powermove ${state.version} is ready` : 'A Powermove update is ready'),
      h('span', `You’re on ${state.current}. Restarting saves your session, installs the update, and reopens Powermove.`),
      h('button.btn', { onclick: installUpdate }, 'Restart to update'));
  };
  S.offAppUpdate = subscribeAppUpdates(paintAppUpdate);
  /* The Store is its own screen; from home it reads as one more place to go. */
  const store = h('div.ps-nav', h('button.ps-navbtn', { onclick: () => (PM as any).StoreUI?.open?.() }, PM.icon('sparkle'), h('span', 'Store')));
  /* One nav block so the highlight can glide from the sections to Store. */
  const nav = h('div.ps-navs', S.nav, store);
  S.offGlide = mountNavGlide(nav, { row: '.ps-navbtn', selected: '.on' });
  /* Who you are on Powermove Cloud, pinned to the foot of the sidebar. */
  S.account = h('div.ps-account');
  S.accountButton = mount(AccountButton, { target: S.account, props: { PM } });
  const sidebar = h('aside.ps-sidebar', h('label.ps-search', PM.icon('search'), S.search), nav, S.appUpdate, S.updates, S.account);

  S.title = h('b'); S.count = h('span');
  const view = h('div.ps-view', { role: 'group', 'aria-label': 'Project layout' },
    viewButton('grid', 'Grid view', 'grid'), viewButton('list', 'List view', 'list'));
  const sort = h('select.ps-sort', { 'aria-label': 'Sort projects' },
    h('option', { value: 'recent' }, 'Recently edited'), h('option', { value: 'name' }, 'Name'));
  sort.value = S.sort;
  sort.onchange = () => { S.sort = sort.value; PM.store.set('projectsSort', S.sort); paint(); };
  const top = h('div.ps-top', h('div.ps-title', S.title, S.count), view, sort,
    h('button.btn', { onclick: openProjectFromDisk }, 'Open Project…'),
    h('button.btn.pri', { onclick: () => { PM.ProjectsScreen.hide(); PM.newProject(); } }, PM.icon('plus'), 'New Project'));
  S.grid = h('div.ps-grid');
  S.pager = h('nav.ps-pagination', { 'aria-label': 'Project pages', hidden: true });
  S.content = h('div.ps-content', S.grid, S.pager);
  S.el = h('div#projects-screen', sidebar, h('main.ps-main', top, S.content));
  document.body.appendChild(S.el);
  /* Only Escape is the home's; app shortcuts such as ⌘, keep working over it. */
  S.el.addEventListener('keydown', (e: any) => {
    if (e.key !== 'Escape') return;
    e.stopPropagation(); e.preventDefault(); PM.ProjectsScreen.hide();
  });
  S.el.addEventListener('dragover', (e: any) => { if ([...e.dataTransfer.types].includes('Files')) e.preventDefault(); });
  S.el.addEventListener('drop', async (e: any) => {
    const file = [...e.dataTransfer.files].find((f: any) => /\.pmv$/i.test(f.name));
    if (!file) return;
    e.preventDefault();
    const before = PM.proj.id;
    await PM.importFiles?.([file]);
    if (PM.proj.id !== before) PM.ProjectsScreen.hide();
  });
}

function viewButton(value: any, label: any, icon: any) {
  const b = h('button', { title: label, 'aria-label': label, onclick: () => {
    S.view = value; PM.store.set('projectsView', value); paint();
  } }, PM.icon(icon));
  b.dataset.view = value;
  return b;
}
function navButton(section: any, label: any, icon: any, count: any) {
  return h('button.ps-navbtn' + (S.section === section ? '.on' : ''), {
    onclick: () => { S.section = section; PM.store.set('projectsSection', section); paint(); },
  }, PM.icon(icon), h('span', label), h('span.count', String(count)));
}

function paint() {
  if (!S.el) return;
  const live = [...PM.Projects.list()], trash = [...PM.Projects.trashList()];
  S.nav.textContent = '';
  S.nav.append(navButton('recents', 'Recents', 'clock', Math.min(12, live.length)),
    navButton('projects', 'Projects', 'project', live.length),
    navButton('trash', 'Trash', 'trash', trash.length));
  const labels: any = { recents: 'Recents', projects: 'Projects', trash: 'Trash' };
  let metas = S.section === 'trash' ? trash : live;
  if (S.sort === 'name') metas.sort((a: any, b: any) => String(a.name).localeCompare(String(b.name)));
  else metas.sort((a: any, b: any) => (b.at || b.deletedAt || 0) - (a.at || a.deletedAt || 0));
  const q = S.search.value.trim().toLowerCase();
  if (q) metas = metas.filter((m: any) => String(m.name || '').toLowerCase().includes(q));
  else if (S.section === 'recents') metas = metas.slice(0, 12);
  const queryKey = JSON.stringify([S.section, S.sort, S.view, q]);
  if (queryKey !== S.queryKey) { S.page = 0; S.queryKey = queryKey; S.content.scrollTop = 0; }
  const pages = Math.max(1, Math.ceil(metas.length / PAGE_SIZE));
  S.page = Math.min(S.page, pages - 1);
  S.title.textContent = labels[S.section];
  S.count.textContent = `${metas.length} ${metas.length === 1 ? 'project' : 'projects'}`;
  S.el.querySelectorAll('.ps-view button').forEach((b: any) => b.classList.toggle('on', b.dataset.view === S.view));
  S.grid.className = 'ps-grid' + (S.view === 'list' ? ' list' : '') + (!metas.length ? ' empty' : '');
  S.grid.textContent = '';
  const openIds = new Set<string>(PM.Projects.openProjects());
  metas.slice(S.page * PAGE_SIZE, (S.page + 1) * PAGE_SIZE)
    .forEach((m: any) => S.grid.appendChild(card(m, S.section === 'trash', openIds)));
  if (!metas.length) S.grid.appendChild(emptyState(q));
  S.pager.textContent = '';
  S.pager.hidden = pages === 1;
  if (pages > 1) {
    const turnPage = (delta: number) => {
      S.page += delta;
      paint();
      S.content.scrollTop = 0;
      S.pager.querySelector('button:not(:disabled)')?.focus();
    };
    S.pager.append(
      h('button.btn', { disabled: S.page === 0, onclick: () => turnPage(-1) }, 'Previous page'),
      h('span', { 'aria-live': 'polite' }, `Page ${S.page + 1} of ${pages}`),
      h('button.btn', { disabled: S.page === pages - 1, onclick: () => turnPage(1) }, 'Next page'));
  }
}

function card(m: any, trashed: any, openIds: Set<string>) {
  const active = !trashed && m.id === PM.proj.id;
  /* "Open" is now a window somewhere, which may be this one or another. */
  const open = !trashed && (active || openIds.has(m.id));
  const file = !trashed ? PM.projectFileState?.(m.id, { initialize: false }) : null;
  // Dimensions belong in the index; cards must never clone whole compositions.
  const raw = m;
  const inner = h('div.ps-thumb-inner');
  const dims = raw && raw.w && raw.h ? `${raw.w}×${raw.h}` : '';
  if (m.thumb) inner.appendChild(h('img', { src: m.thumb, alt: '', loading: 'lazy', decoding: 'async' }));
  /* An unrendered project shows its canvas size in the frame instead of a blank slab. */
  const thumb = h('div.ps-thumb', inner, !m.thumb && dims ? h('div.ps-thumb-empty', dims) : null);
  const more = h('button.ps-more', { title: 'Project actions', 'aria-label': 'Project actions' }, PM.icon('more'));
  const sub = projectSub(m, raw, trashed, file);
  /* Where the project is (Active, Open) is a quiet tag beside the name, so
     every card keeps one silhouette and the frame stays clean. */
  const tag = active || open ? h('span.ps-tag', active ? 'This window' : 'Open') : null;
  const meta = h('div.ps-meta', h('div.ps-meta-copy',
    h('div.ps-name-row', h('div.ps-name', { title: m.name || 'Untitled' }, m.name || 'Untitled'), tag),
    h('div.ps-sub', { title: sub.title }, sub.text)), more);
  const c = h('article.ps-card' + (active ? '.active' : ''), thumb, meta);
  c.onclick = () => {
    if (trashed) return;
    openLocalProject(m);
  };
  c.oncontextmenu = (e: any) => { e.preventDefault(); projectMenu(c, m, trashed, e.clientX, e.clientY); };
  more.onclick = (e: any) => { e.stopPropagation(); projectMenu(more, m, trashed); };
  return c;
}
function projectSub(m: any, raw: any, trashed: any, file: any) {
  const dims = raw && raw.w && raw.h ? `${raw.w}×${raw.h}` : '';
  const date = trashed ? `Deleted ${ago(m.deletedAt)}` : `Edited ${ago(m.at)}`;
  if (trashed) return { text: dims ? `${date} · ${dims}` : date, title: date };
  /* The date leads because it is what people scan for; the file only speaks
     up when there is one (name, or unsaved changes). Size fits when room allows. */
  const location = file?.path ? file.path.split(/[\\/]/).pop() : '';
  const status = file?.path ? (file.dirty ? `Unsaved changes · ${location}` : location) : '';
  return {
    text: [date, status, dims].filter(Boolean).join(' · '),
    title: file?.path ? `${file.dirty ? 'Unsaved changes' : 'Saved'}\n${file.path}` : 'Not saved to a file',
  };
}
function projectMenu(anchor: any, m: any, trashed: any, x?: any, y?: any) {
  const items = trashed ? [
    { label: 'Restore', run: () => restore(m) },
    '-', { label: 'Delete Forever…', run: () => destroyDialog(m) },
  ] : [
    { label: 'Open', disabled: m.id === PM.proj.id, run: () => openLocalProject(m) },
    ...(PM.windows?.supported
      ? [{ label: 'Open in New Window', disabled: m.id === PM.proj.id, run: () => openInNewWindow(m) }]
      : []),
    { label: 'Save', run: () => save(m, false) },
    { label: 'Save As…', run: () => save(m, true) },
    '-',
    { label: 'Rename…', run: () => renameDialog(m) }, { label: 'Duplicate', run: () => duplicate(m) },
    '-', { label: 'Move to Trash…', run: () => trashDialog(m) },
  ];
  PM.menu(anchor, items, x == null ? {} : { x, y });
}

async function openProjectFromDisk() {
  const before = PM.proj.id;
  await PM.openProject?.();
  if (PM.proj.id !== before) PM.ProjectsScreen.hide();
}
/* A click loads the project into this window. A document lives in one window at
   a time, so one already open elsewhere brings its own window forward instead
   and this window keeps what it had. */
function openLocalProject(m: any) {
  if (m.id === PM.proj.id) {
    PM.ProjectsScreen.hide();
    return;
  }
  void Promise.resolve(PM.openProjectHere?.(m.id) ?? false).then((opened: boolean) => {
    if (opened) PM.ProjectsScreen.hide();
    else paint();
  });
}
function openInNewWindow(m: any) {
  void Promise.resolve(PM.openProjectInNewWindow?.(m.id) ?? false).then(() => paint());
}
async function save(m: any, saveAs: boolean) {
  if (!PM.Projects.get(m.id)) return PM.toast('Could not save this project because its local data is missing.');
  await PM.saveProject?.({ projectId: m.id, saveAs });
  paint();
}
function restore(m: any) {
  if (!PM.Projects.restore(m.id)) return PM.toast('Could not restore this project because its local data is missing.');
  paint(); PM.bus.emit('projects:open'); PM.toast('Project restored');
}

function emptyState(searching: any) {
  const trash = S.section === 'trash';
  const title = searching ? 'No matching projects' : trash ? 'Trash is empty' : 'No projects yet';
  const body = searching ? 'Try a different name.' : trash
    ? 'Projects moved to Trash stay here until you restore or permanently delete them.'
    : 'Projects keep the composition, layers, reusable sections and exports together.';
  return h('div.ps-empty', PM.icon(trash ? 'trash' : searching ? 'search' : 'project'), h('b', title), h('span', body),
    !searching && !trash ? h('button.btn', { onclick: () => { PM.ProjectsScreen.hide(); PM.newProject(); } }, PM.icon('plus'), 'Create a project') : null);
}

function renameDialog(m: any) {
  const raw = PM.Projects.get(m.id), name = h('input', { value: raw && raw.name || m.name });
  PM.modal({ title: 'Rename project', body: h('div.field', name), width: 400, actions: [
    { label: 'Cancel' }, { label: 'Rename', pri: true, run: () => {
      try {
        if (!PM.Projects.rename(m.id, name.value)) throw new Error('Project data is missing');
        paint(); PM.bus.emit('projects:open'); PM.bus.emit('project');
      } catch (error: any) { PM.toast('Could not rename project: ' + (error.message || 'Unknown error')); }
    } },
  ] });
  window.setTimeout(() => { name.focus(); name.select(); }, 30);
}
function duplicate(m: any) {
  const source = PM.Projects.get(m.id);
  if (!source) return PM.toast('Project data missing');
  try {
    const raw = JSON.parse(JSON.stringify(source));
    raw.id = PM.uid('P'); raw.name = (m.name || 'Untitled') + ' copy';
    PM.Projects.put(raw, m.thumb);
    const sourceState = PM.Projects.getState(m.id);
    if (sourceState) {
      const copiedState = JSON.parse(JSON.stringify(sourceState));
      delete copiedState.file;
      PM.Projects.putState(raw.id, copiedState);
    }
    paint(); PM.bus.emit('projects:open'); PM.toast('Duplicated “' + m.name + '”', 2200, { error: false });
  } catch (error: any) { PM.toast('Could not duplicate project: ' + (error.message || 'Unknown error')); }
}
function trashDialog(m: any): Promise<void> {
  return PM.confirm({ message: 'Move “' + m.name + '” to Trash?', detail: 'You can restore this project from Trash.', confirmLabel: 'Move to Trash' }).then(async (ok: boolean) => {
    if (!ok) return;
    if (m.id === PM.proj.id) {
      try {
        // Save the live document and its Undo/session state before removing
        // its registry entry. The following project switch must not put it back.
        await PM.flushProject();
      } catch (error: any) {
        PM.toast('Could not move this project to Trash: ' + (error?.message || 'Project storage is unavailable'));
        return;
      }
    }
    if (!PM.Projects.trash(m.id)) return PM.toast('Could not move this project to Trash.');
    if (m.id === PM.proj.id) switchUnderlying(); paint(); PM.bus.emit('projects:open');
  });
}
function destroyDialog(m: any): Promise<void> {
  return PM.confirm({ message: 'Delete “' + m.name + '” forever?', detail: 'This permanently removes the local project. This cannot be undone.', confirmLabel: 'Delete Forever', destructive: true })
    .then((ok: boolean) => { if (ok) { PM.Projects.destroy(m.id); paint(); } });
}
/* Trashing the composition this window is editing leaves it with nothing, so it
   falls back to another project that no other window holds, or a blank one. */
function switchUnderlying() {
  const taken = new Set(PM.Projects.openProjects().filter((id: string) => id !== PM.proj.id));
  const id = PM.Projects.list().map((m: any) => m.id).find((candidate: string) => !taken.has(candidate));
  const p = id && PM.Projects.get(id);
  window.dispatchEvent(new window.CustomEvent('pm-open-project', { detail: p || PM.mkProject({ name: 'Untitled', dur: 10, w: 1920, h: 1080, fps: 30, bg: '#09090A' }) }));
}
function ago(t: any) {
  const s = Math.max(1, (Date.now() - (t || 0)) / 1000);
  if (s < 90) return 'just now'; if (s < 3600) return Math.round(s / 60) + ' min ago';
  if (s < 86400) return Math.round(s / 3600) + ' h ago'; if (s < 604800) return Math.round(s / 86400) + ' d ago';
  return new Date(t).toLocaleDateString();
}

const offOpen = PM.bus.on('projects:open', () => { if (PM.ProjectsScreen.isOpen) paint(); });
PM.__disposeProjectsScreen = () => {
  S.offUpdates?.(); S.offUpdates = null;
  S.offAppUpdate?.(); S.offAppUpdate = null;
  S.offGlide?.(); S.offGlide = null;
  if (S.accountButton) void unmount(S.accountButton); S.accountButton = null;
  offOpen?.();
  S.el?.remove?.();
  S.el = null;
};
if (wasOpen) window.queueMicrotask(() => PM.ProjectsScreen.show(previousSection));
}

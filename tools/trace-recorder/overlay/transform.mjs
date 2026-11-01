import ts from 'typescript';
import path from 'node:path';

const ROOT = 'src/renderer/src/';
export const targets = ['legacy/core/history.ts', 'legacy/core/editing.ts', 'legacy/core/workspace.ts',
  'legacy/core/anim.ts', 'legacy/core/util.ts', 'legacy/ui/shortcuts.ts', 'legacy/host/electron-shim.ts',
  'kernel/install.ts', 'kernel/registries.ts', 'controls/gesture.ts'];
function enclosing(node, predicate) {
  for (let p = node.parent; p; p = p.parent) if (predicate(p)) return p;
}
export function transform(source, file) {
  const sf = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const edits = [];
  const insert = (pos, text) => edits.push({ start: pos, end: pos, text });
  const replace = (start, end, text) => edits.push({ start, end, text });
  const once = (needle, text) => {
    const index = source.indexOf(needle);
    if (index < 0 || source.indexOf(needle, index + 1) >= 0) throw new Error(`Overlay anchor not unique: ${file}: ${needle}`);
    insert(index, text);
  };
  const wrap = (n, name) => {
    if(n.parameters.some(p=>!ts.isIdentifier(p.name))) throw new Error(`Unsupported destructured trace arguments: ${file}`);
    const args = !ts.isArrowFunction(n) ? '...arguments' : n.parameters.map(p => `${p.dotDotDotToken ? '...' : ''}${p.name.getText(sf)}`).join(', ');
    if (ts.isBlock(n.body)) {
      insert(n.body.getStart(sf) + 1, `return __pmTrace.call(${JSON.stringify(name)}, [${args}], () => {`);
      insert(n.body.end - 1, '});');
    } else {
      insert(n.body.getStart(sf), `__pmTrace.call(${JSON.stringify(name)}, [${args}], () => (`);
      insert(n.body.end, '))');
    }
  };
  const nameOf = n => n.name?.getText(sf);
  const property = n => { let p=n.parent; while (ts.isAsExpression(p) || ts.isParenthesizedExpression(p)) p=p.parent; return ts.isPropertyAssignment(p) ? p : null; };
  const variable = (n, name) => enclosing(n, p => ts.isVariableDeclaration(p) && nameOf(p) === name);
  const factory = (n, name) => enclosing(n, p => ts.isFunctionDeclaration(p) && nameOf(p) === name);
  const visit = n => {
    if ((ts.isMethodDeclaration(n) || ts.isArrowFunction(n) || ts.isFunctionDeclaration(n)) && n.body) {
      const name = nameOf(n) ?? (property(n) ? nameOf(property(n)) : undefined);
      if (file === 'legacy/core/history.ts' && ts.isMethodDeclaration(n) && variable(n, 'H') && n.parent === variable(n, 'H').initializer && ['begin','beginScoped','track','hasChanges','commit','cancel','rollback','do','undo','redo','external','selection','undoIfTop','clear'].includes(name))
        wrap(n, `history.${name}`);
      if (file === 'legacy/core/editing.ts' && ts.isMethodDeclaration(n) && variable(n, 'Edit') && ['apply','mutate','begin','dispatch','commit','cancel'].includes(name))
        wrap(n, `edit.${name}`);
      if (file === 'controls/gesture.ts' && ts.isMethodDeclaration(n)) {
        wrap(n, `gesture.${name}`);
        insert(n.body.getStart(sf)+1, '__pmTrace.point("gesture.context", {gesture:this, binding:this.b});');
      }
      if (file === 'kernel/install.ts' && ts.isArrowFunction(n) && (factory(n,'makeHistory') || factory(n,'makeEdit') || (factory(n,'makeWorkspace') && name==='mutate')) && property(n))
        wrap(n, `api.${factory(n,'makeHistory') ? 'history' : factory(n,'makeEdit') ? 'edit' : 'workspace'}.${name}`);
      if (ts.isArrowFunction(n) && ts.isBinaryExpression(n.parent)) {
        const left = n.parent.left.getText(sf);
        if (file === 'legacy/core/workspace.ts' && left === 'WS.mutate') wrap(n,'workspace.mutate');
        if (file === 'legacy/core/anim.ts' && left === 'PM.touch') wrap(n,'PM.touch');
        if (file === 'legacy/ui/shortcuts.ts' && left === 'PM.cmd') wrap(n,'command.run');
        if (file === 'legacy/core/util.ts' && left === 'PM.uid') {
          insert(n.body.getStart(sf),'__pmTrace.uid('); insert(n.body.end,')');
        }
      }
      if (file === 'kernel/registries.ts') {
        if (name === 'emit') insert(n.body.getStart(sf)+1, '__pmTrace.point("event.kernel", {event, payload});');
        if (name === 'handler' || (ts.isArrowFunction(n) && ts.isVariableDeclaration(n.parent) && nameOf(n.parent)==='handler')) wrap(n,'keymap.bubble');
        if (name === 'bindingsFor') wrap(n,'keymap.resolve');
        if (name === 'dispatchTrustedKey') wrap(n,'keymap.trusted');
        if (name === 'runKernelCommand') wrap(n,'command.kernel');
      }
    }
    if (file === 'kernel/install.ts' && ts.isReturnStatement(n) && n.expression && ['makeHistory','makeEdit'].some(x=>factory(n,x))) {
      const which = factory(n,'makeHistory') ? 'history' : 'edit';
      insert(n.expression.getStart(sf),`__pmTrace.expose('${which}', `); insert(n.expression.end,')');
    }
    ts.forEachChild(n,visit);
  };
  visit(sf);
  if (file === 'legacy/core/history.ts') once('PM.hist = H;', `__pmTrace.attach(PM, () => ({depth, index: idx, pending: pending ? {label: pending.label, scoped: !!pending.scoped, before: pending.before, scopes: pending.scopes} : null, entries: stack.map(e => ({id:e.id, label:e.label, project:!!e.project, forward:e.forward, backward:e.backward, undo:e.undo, redo:e.redo, cleanup:e.cleanup})), totalBytes}));\n`);
  if (file === 'controls/gesture.ts') insert(source.length,'\n__pmTrace.gesture = EditGesture;\n');
  if (file === 'kernel/install.ts') {
    const needle = 'workspace: makeWorkspace(PM),';
    once(needle,'');
    replace(source.indexOf(needle), source.indexOf(needle)+needle.length, "workspace: __pmTrace.expose('workspace', makeWorkspace(PM)),");
  }
  if (file === 'kernel/registries.ts') {
    once('if (field && !binding.inFields) continue;', '__pmTrace.point("key.candidate", {binding, field, repeat:event.repeat, defaultPrevented:event.defaultPrevented, skip: field && !binding.inFields ? "field" : event.repeat && !binding.repeat ? "repeat" : null});\n');
    once('if (result === false) continue;', '__pmTrace.point("key.result", {binding, result, defaultPrevented:event.defaultPrevented});\n');
  }
  if (file === 'legacy/host/electron-shim.ts') once("const editorCommand = command === 'copy'", '__pmTrace.point("menu.received", {command});\n');
  if (file === 'legacy/core/util.ts') {
    const marker='bus.emit = (ev: any, a: any, b: any) => {';
    once(marker, '');
    insert(source.indexOf(marker)+marker.length, '__pmTrace.point("event.legacy", {event:ev, args:[a,b]});');
  }
  edits.sort((a,b)=>b.start-a.start || b.end-a.end);
  for (const edit of edits) source=source.slice(0,edit.start)+edit.text+source.slice(edit.end);
  const runtime = path.posix.relative(path.posix.dirname(file), 'trace-recorder/runtime').replace(/^(?!\.)/, './');
  return `import { trace as __pmTrace } from ${JSON.stringify(runtime)};\n${source}`;
}
export function traceRecorderPlugin() {
  return {name:'powermove-reference-trace-recorder', enforce:'pre', transform(source,id) {
    if (process.env.POWERMOVE_TRACE_RECORDER !== '1') return;
    const file = id.replaceAll('\\','/').split(ROOT)[1];
    if (!targets.includes(file)) return;
    return {code:transform(source,file),map:null};
  }};
}

export function traceRecorderMainPlugin() {
  return {name:'powermove-reference-main-trace',enforce:'pre',transform(source,id) {
    if(process.env.POWERMOVE_TRACE_RECORDER!=='1'||!id.replaceAll('\\','/').endsWith('/src/main/menu.ts')) return;
    const anchor='webContents.setIgnoreMenuShortcuts(isRendererOwnedMenuInput(input));';
    if(!source.includes(anchor)) throw new Error('Main menu routing anchor missing');
    source=source.replace(anchor,`const owned = isRendererOwnedMenuInput(input);
      __pmMainTrace.push({type:'menu.routing',input:{type:input.type,key:input.key,code:input.code,meta:input.meta,control:input.control,alt:input.alt,shift:input.shift,isAutoRepeat:input.isAutoRepeat},ignoreMenuShortcuts:owned});
      webContents.setIgnoreMenuShortcuts(owned);`);
    if(!source.includes('target.webContents.send(IPC.menuCommand, command);'))throw new Error('Main menu send anchor missing');
    source=source.replace('target.webContents.send(IPC.menuCommand, command);', `__pmMainTrace.push({type:'menu.send',command}); target.webContents.send(IPC.menuCommand, command);`);
    return {code:`const __pmMainTrace: any[] = (globalThis as any).__PMTraceMain = [];\n`+source,map:null};
  }};
}

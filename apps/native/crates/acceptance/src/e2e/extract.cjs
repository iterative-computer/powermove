#!/usr/bin/env node
// Development-only TypeScript syntax inspection; never execute the old app.
// Install typescript@5.9.3 in work/e2e and set NODE_PATH to that node_modules.
const ts = require('typescript');
const cp = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
if (ts.version !== '5.9.3') throw Error('Expected TypeScript 5.9.3');
const root = cp.execFileSync('git', ['rev-parse', '--show-toplevel'], {encoding:'utf8'}).trim();
process.chdir(root);
const flags=process.argv.slice(2);
if (flags.some(flag=>!['--write','--reference=main','--reference=v1.2.0'].includes(flag)) || (flags.includes('--reference=main') && flags.includes('--reference=v1.2.0'))) throw Error('Expected --reference=main or --reference=v1.2.0');
const mainReference = !flags.includes('--reference=v1.2.0');
const revision = mainReference ? 'c7f46a05641b23944d3692c9d80f9677c7e1ef1d' : '07ed5fa17ee640659cb075f9cad1af2095011d60';
if (!mainReference && cp.execFileSync('git', ['rev-parse', 'v1.2.0^{commit}'], {encoding:'utf8'}).trim() !== revision) {
  throw Error('v1.2.0 is not the frozen reference');
}
const files = cp.execFileSync('git', ['grep', '-l', 'test(', revision, '--', 'apps/desktop/e2e'], {encoding:'utf8'})
  .trim().split('\n').map(f => f.slice(revision.length+1));
const hash = s => crypto.createHash('sha256').update(s).digest('hex');
const cases = [], sources = [];
for (const file of files) {
  const source = cp.execFileSync('git', ['show', `${revision}:${file}`], {encoding:'utf8',maxBuffer:10*1024*1024});
  const ast = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  if (ast.parseDiagnostics.length) throw Error(file + ': TypeScript parse error');
  let count = 0;
  const fileGuards = [];
  const definitions = new Map();
  function definitionsWalk(n) {
    if (ts.isVariableDeclaration(n) && ts.isIdentifier(n.name) && n.initializer) {
      const name = n.name.text;
      if (!definitions.has(name)) definitions.set(name, []);
      definitions.get(name).push(n.getText(ast));
    }
    ts.forEachChild(n, definitionsWalk);
  }
  definitionsWalk(ast);
  function walk(n) {
    if (ts.isCallExpression(n) && /^test\.(skip|fixme|fail)$/.test(n.expression.getText(ast))) {
      fileGuards.push(n.getText(ast));
    }
    if (ts.isCallExpression(n) && /^test(?:\.(?:only|skip|fixme|fail))?$/.test(n.expression.getText(ast)) &&
        n.arguments.length >= 2 && n.arguments.some((a, i) => i > 0 && (ts.isArrowFunction(a) || ts.isFunctionExpression(a)))) {
      const title = n.arguments[0];
      if (!ts.isStringLiteralLike(title) && !ts.isTemplateExpression(title)) throw Error(file + ': unhandled title expression');
      const line = ast.getLineAndCharacterOfPosition(n.getStart(ast)).line + 1;
      const parameters = [], parameterDefinitions = [];
      for (let p = n.parent; p; p = p.parent) {
        if (ts.isForOfStatement(p)) {
          parameters.unshift(p.initializer.getText(ast) + ' of ' + p.expression.getText(ast));
          // Preserve source definitions for named tables without evaluating JS.
          // Multiple candidates are retained explicitly rather than guessing scope.
          if (ts.isIdentifier(p.expression)) parameterDefinitions.push(...(definitions.get(p.expression.text) || []));
        }
      }
      const guards = [];
      function guardsWalk(x) {
        if (ts.isCallExpression(x) && /^test\.(skip|fixme|fail)$/.test(x.expression.getText(ast))) guards.push(x.getText(ast));
        ts.forEachChild(x, guardsWalk);
      }
      guardsWalk(n);
      cases.push({id:`${file}#L${line}`,file,line,title:ts.isStringLiteralLike(title)?title.text:title.getText(ast),
        registration:n.expression.getText(ast),parameters,parameterDefinitions,guards,sha256:hash(n.getText(ast))});
      count++;
    }
    ts.forEachChild(n, walk);
  }
  walk(ast);
  sources.push({file,sha256:hash(source),declarations:count,guards:fileGuards});
}
const result = {revision,sources,cases};
const destination = path.join(__dirname, mainReference ? '../../data/main-e2e-frozen.json' : 'frozen.json');
const output = JSON.stringify(result, null, 2) + '\n';
if (process.argv.includes('--write')) fs.writeFileSync(destination, output);
else if (fs.readFileSync(destination, 'utf8') !== output) throw Error('Frozen inventory drift; inspect before extract.cjs --write');
console.log(JSON.stringify({files:sources.length,specFiles:sources.filter(s=>s.declarations).length,
  declarations:cases.length,parameterizedDeclarations:cases.filter(c=>c.parameters.length).length,
  nonTests:sources.filter(s=>!s.declarations).map(s=>s.file)}));

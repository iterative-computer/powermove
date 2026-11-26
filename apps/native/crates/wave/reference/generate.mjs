// Run from a scratch directory OUTSIDE the repository. See ../README.md.
// Only public upstream code and our synthetic test images enter the browser.
import fs from 'node:fs/promises';
import path from 'node:path';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';
const [upstream, scratch, executablePath] = process.argv.slice(2);
if (!upstream || !scratch) throw new Error('Usage: node generate.mjs UPSTREAM_PACKAGE SCRATCH [HEADLESS_CHROMIUM]');
const require=createRequire(path.resolve(scratch,'package.json'));
const {build}=require('esbuild');
const {chromium}=require('playwright');
const here=path.dirname(fileURLToPath(import.meta.url));
await fs.mkdir(scratch,{recursive:true});
for (const name of ['from','to']) await fs.copyFile(path.join(here,'../tests/fixtures',`${name}.png`),path.join(scratch,`${name}.png`));
await fs.copyFile(path.join(here,'index.html'),path.join(scratch,'index.html'));
await build({entryPoints:[path.join(here,'page.tsx')],bundle:true,format:'esm',outfile:path.join(scratch,'page.js'),alias:{'@upstream':path.resolve(upstream,'src'),react:path.join(scratch,'node_modules/react'),'react-dom':path.join(scratch,'node_modules/react-dom')}});
const browser=await chromium.launch({headless:true,...(executablePath?{executablePath}:{}),args:['--allow-file-access-from-files','--enable-webgl','--use-angle=metal']});
try {
 const page=await browser.newPage({viewport:{width:620,height:400},deviceScaleFactor:1});
 page.on('pageerror',e=>{throw e});
 await page.goto(`file://${path.resolve(scratch,'index.html')}`);
 await page.waitForFunction(()=>window.setFrame);
 await fs.mkdir(path.join(scratch,'frames'),{recursive:true});
 for(const type of ['vertical','horizontal','center'])for(const palette of ['colorful','green','ocean','sunset','mono','#528dff'])for(let i=0;i<=10;i++){
  await page.evaluate(async([t,p,s])=>window.setFrame(t,p,s),[type,palette,i/10]);
  await page.screenshot({path:path.join(scratch,'frames',`${type}-${palette.replace('#','')}-${i}.png`)});
 }
 console.log('Captured 198 reference frames at 620×400, DPR 1.');
} finally {await browser.close();}

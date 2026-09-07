import { expect, test } from './helpers/app';
test('group drop previews without mutation and moves selected layers with undo', async ({session})=>{
 const {page}=session;
 await page.waitForFunction(()=>Boolean((window as any).PM?.TL?.cv));
 await page.evaluate(()=>{const P=(window as any).PM,p=P.mkProject({w:640,h:360,dur:5});p.layers=['A','B'].map(name=>P.mkLayer('shape',{name},p));p.layers.push(P.mkLayer('group',{name:'Folder'},p));P.replaceProject(p);P.selectLayers(p.layers.slice(0,2).map((l:any)=>l.id));P.invalidate();});
 await page.waitForFunction(()=>(window as any).PM.TL.rows.length===3);
 const b=await page.evaluate(()=>{const T=(window as any).PM.TL,r=T.cv.getBoundingClientRect();return {x:r.left+110,y:r.top+T.ruler+T.row/2,row:T.row};});
 await page.mouse.move(b.x,b.y);await page.mouse.down();await page.mouse.move(b.x,b.y+b.row*2,{steps:8});
 expect(await page.evaluate(()=>(window as any).PM.TL.reorder?.mode)).toBe('inside');
 expect(await page.evaluate(()=>(window as any).PM.proj.layers[0].group||null)).toBeNull();
 await page.screenshot({path:'/tmp/powermove-layer-reorder-preview.png'});
 await page.mouse.up();
 expect(await page.evaluate(()=>{const P=(window as any).PM;return P.proj.layers.filter((l:any)=>l.name!=='Folder').every((l:any)=>l.group===P.proj.layers.find((g:any)=>g.name==='Folder').id);})).toBe(true);
 await page.evaluate(()=>(window as any).PM.hist.undo());
 expect(await page.evaluate(()=>(window as any).PM.proj.layers.map((l:any)=>l.name))).toEqual(['A','B','Folder']);
 expect(await page.evaluate(()=>(window as any).PM.proj.layers.every((l:any)=>!l.group))).toBe(true);
 expect(session.diagnostics.pageErrors).toEqual([]);
});

test('selection and same-group reordering undo and redo in order', async ({session})=>{
 const {page}=session;
 await page.waitForFunction(()=>Boolean((window as any).PM?.TL?.cv));
 await page.evaluate(()=>{const P=(window as any).PM,p=P.mkProject({w:640,h:360,dur:5});p.layers=['A','B','C'].map(name=>P.mkLayer('shape',{name},p));P.replaceProject(p);P.selectLayers(p.layers[0].id);P.hist.clear();P.invalidate();});
 await page.waitForFunction(()=>(window as any).PM.TL.rows.length===3);
 const b=await page.evaluate(()=>{const T=(window as any).PM.TL,r=T.cv.getBoundingClientRect();return {x:r.left+110,y:r.top+T.ruler+T.row/2,row:T.row};});
 await page.mouse.click(b.x,b.y+b.row);
 await page.evaluate(()=>(window as any).PM.hist.undo());
 expect(await page.evaluate(()=>(window as any).PM.firstSel().name)).toBe('A');
 await page.evaluate(()=>(window as any).PM.hist.redo());
 expect(await page.evaluate(()=>(window as any).PM.firstSel().name)).toBe('B');
 await page.mouse.move(b.x,b.y+b.row);await page.mouse.down();await page.mouse.move(b.x,b.y+2.4*b.row,{steps:8});await page.mouse.up();
 const order=()=>page.evaluate(()=>(window as any).PM.proj.layers.map((l:any)=>l.name));
 expect(await order()).toEqual(['A','C','B']);
 await page.evaluate(()=>(window as any).PM.hist.undo());expect(await order()).toEqual(['A','B','C']);
 await page.evaluate(()=>(window as any).PM.hist.redo());expect(await order()).toEqual(['A','C','B']);
 expect(session.diagnostics.pageErrors).toEqual([]);
});

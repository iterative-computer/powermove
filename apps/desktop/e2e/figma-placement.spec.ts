import { test, expect } from './helpers/app';
import { validateFigmaDoc, type FigmaNode } from '../src/shared/figma/doc';

const solid = (r: number, g: number, b: number, a = 1) => ({type: 'SOLID', color: {r,g,b,a}});
const node = (id: string, patch: Partial<FigmaNode>): FigmaNode => ({
  id, fileKey: 'synthetic-placement', name: id, type: 'RECTANGLE', w: 40, h: 30,
  absoluteTransform: [1,0,0,1,1000,-500], fills: [solid(1,0,0)], strokes: [], effects: [], children: [], ...patch,
});
test('first import retains nested, rotated and reflected artwork and paint order', async ({session}) => {
  await session.openEditor();
  await session.page.evaluate(() => {
    const P=(window as any).PM;
    P.replaceProject(P.mkProject({w:360,h:240,dur:4,bg:'#272727'}));P.setTime(0);
  });
  const doc=validateFigmaDoc({format:'powermove-figma',version:1,source:'plugin',images:{},nodes:[
    node('screen',{type:'FRAME',w:300,h:180,fills:[solid(.92,.92,.92)],children:[
      node('rotated group',{type:'GROUP',w:80,h:80,fills:[],absoluteTransform:[.8660254,.5,-.5,.8660254,1100,-450],children:[
        node('overlaid fills',{absoluteTransform:[.8660254,.5,-.5,.8660254,1110.980762,-409.019238],fills:[solid(0,0,1),solid(0,1,0,.75)]}),
      ]}),
      node('reflected vector',{type:'VECTOR',absoluteTransform:[-1,0,0,1,1290,-480],paths:[{data:'M0 0 L40 0 L10 30 Z',windingRule:'NONZERO'}]}),
    ]}),
  ]});
  const result=await session.app.evaluate(async({ipcMain,BrowserWindow}:any,doc:any)=>new Promise<any>((resolve,reject)=>{
    const id='placement';const timer=setTimeout(()=>reject(Error('Import timeout')),15000);
    ipcMain.once('figma:result',(_:any,v:any)=>{clearTimeout(timer);resolve(v.result)});
    BrowserWindow.getAllWindows()[0].webContents.send('figma:receive',{id,doc});
  }),doc);
  expect(result.error).toBeUndefined();expect(result.flattened).toBe(0);
  const error=await session.page.evaluate(()=>{
    const P=(window as any).PM,actual=P.renderFrameTo(0,360,240,{mblur:false}).getContext('2d').getImageData(0,0,360,240).data;
    const ref=document.createElement('canvas');ref.width=360;ref.height=240;const c=ref.getContext('2d')!;
    c.fillStyle='#272727';c.fillRect(0,0,360,240);c.fillStyle='rgb(235,235,235)';c.fillRect(30,30,300,180);
    // Source's absolute transforms, with one selection-wide translation.
    c.setTransform(.8660254,.5,-.5,.8660254,140.980762,120.980762);
    c.fillStyle='blue';c.fillRect(0,0,40,30);c.fillStyle='rgba(0,255,0,.75)';c.fillRect(0,0,40,30);
    c.setTransform(-1,0,0,1,320,50);c.fillStyle='red';c.fill(new Path2D('M0 0 L40 0 L10 30 Z'));
    const expected=c.getImageData(0,0,360,240).data;let sum=0;
    for(let i=0;i<actual.length;i+=4)for(let k=0;k<3;k++)sum+=Math.abs(actual[i+k]-expected[i+k]);
    return sum/(360*240*3);
  });
  expect(error).toBeLessThan(.3);expect(session.diagnostics.pageErrors).toEqual([]);
});

import {test,expect} from './helpers/app';
import {readFileSync} from 'node:fs';
test('orientation keeps coplanar group text and details above the card background',async({session})=>{
 const {page}=session;await page.waitForFunction(()=>Boolean((window as any).PM?.GL?.gl));
 const specimen=process.env.POWERMOVE_25D_SPECIMEN ? JSON.parse(readFileSync(process.env.POWERMOVE_25D_SPECIMEN,'utf8')).proj : null;
 const result=await page.evaluate((specimen)=>{
  const PM=(window as any).PM;
  let project,group;
  if(specimen){project=specimen;group=project.layers.find((l:any)=>l.name==='Remote Desktop');project.layers=project.layers.filter((l:any)=>l.id===group.id||l.group===group.id);}
  else {
   project=PM.mkProject({name:'Coplanar group',w:640,h:360,dur:3,bg:'#EEEEEE'});
   const title=PM.mkLayer('text',{name:'Heading',d:{text:'Remote Desktop',size:36,color:'#111111'},p:{'position.x':180,'position.y':120}},project);
   const detail=PM.mkLayer('shape',{name:'Blue detail',d:{w:80,h:50,color:'#2040FF'},p:{'position.x':350,'position.y':220}},project);
   const card=PM.mkLayer('shape',{name:'White background',d:{w:440,h:260,color:'#FFFFFF'},p:{'position.x':320,'position.y':180}},project);
   project.layers=[title,detail,card];PM.replaceProject(project);group=PM.groupLayers(project.layers.map((l:any)=>l.id));project=PM.proj;
  }
  PM.replaceProject(project);group=PM.L(group.id);group.threeD=true;
  const counts:any[]=[];
  for(const axis of specimen ? ['orientation.x'] : ['orientation.x','orientation.y'])for(const angle of [0,10,-10,30,-30]){
   group.p['orientation.x'].v=0;group.p['orientation.y'].v=0;group.p[axis].v=angle;PM.touch();PM.setTime(2.4333333333,{force:true});
   const cv=PM.renderFrameTo(PM.time,640,360),data=cv.getContext('2d').getImageData(0,0,640,360).data;let dark=0,blue=0;
   for(let i=0;i<data.length;i+=4){if(data[i]<70&&data[i+1]<70&&data[i+2]<70)dark++;if(data[i+2]>180&&data[i]<100&&data[i+1]<130)blue++;}
   counts.push({axis,angle,dark,blue});
  }
  group.p['orientation.y'].v=0;group.p['orientation.x'].v=-10;PM.touch();PM.selectLayers(group.id);PM.invalidate();return counts;
 },specimen);
 console.log('orientation pixel counts',result);
 await page.screenshot({path:process.env.POWERMOVE_25D_SPECIMEN?'/tmp/powermove-orientation-specimen.png':'/tmp/powermove-orientation-regression.png'});
 for(const frame of result){expect(frame.dark,`Text at ${frame.angle} degrees`).toBeGreaterThan(100);if(!specimen)expect(frame.blue,`Detail at ${frame.angle} degrees`).toBeGreaterThan(100);}
 expect(session.diagnostics.pageErrors).toEqual([]);
});

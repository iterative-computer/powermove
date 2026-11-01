async function lifecycleJob(){
 const canvas=document.createElement('canvas');canvas.width=1920;canvas.height=1080;const ctx=canvas.getContext('2d',{willReadFrequently:true});
 for(let phase=1;phase<=3;phase++){
  const next=await (await fetch(base+'life-next?'+(phase-1))).json();if(next.phase!==phase)throw Error('lifecycle phase');
  if(testCase.startsWith('lifecycle-s5b-')){
   for(const component of ['js','scheme','canvas','ids']){
    let n=0,sum=0;const t=performance.now();
    while(performance.now()-t<1000){
     if(component==='js'){for(let i=0;i<10000;i++)sum=(sum+i)%100003;}
     else if(component==='scheme'){if(idSync('id')!=='surface:pattern:7')throw Error('life id');}
     else if(component==='ids'){idCapture();}
     else {ctx.fillStyle='#202126';ctx.fillRect(0,0,1920,1080);for(let i=0;i<100;i++)ctx.fillRect((i*97+n)%1920,i*23%1080,100,50);sum+=ctx.getImageData(0,0,1,1).data[0];}
     n++;
    }
    await report({test:'life_component',phase,component,n,fps:n*1000/(performance.now()-t),checksum:sum,document_hidden:document.hidden});
   }
  }
  const start=performance.now();let frames=0,checksum=0;
  while(performance.now()-start<5000){
   ctx.fillStyle='#202126';ctx.fillRect(0,0,1920,1080);
   for(let i=0;i<100;i++){ctx.fillStyle=`rgb(${i*17%256},${frames%256},90)`;ctx.fillRect((i*97+frames)%1920,i*23%1080,100,50);}
   checksum+=ctx.getImageData(0,0,1,1).data[0];if(testCase.startsWith('lifecycle-s5b-'))idSync('id');else sync('ping');frames++;
  }
  await report({test:'page_job_throughput',phase,frames,elapsed_ms:performance.now()-start,fps:frames*1000/(performance.now()-start),checksum,document_hidden:document.hidden});
  await fetch(base+'life-ready');
 }
}
async function infiniteLoop(){
 if(__RESTARTED__){
  const storage=localStorage.getItem('s5-synthetic-storage')==='kept';
  const snapshot=JSON.parse(sync('kernel-snapshot'));
  if(!storage||snapshot.revision!==7||!snapshot.acknowledged)throw Error('restart state');
  await report({test:'restart_ready',storage_preserved:storage,kernel_snapshot_revision:snapshot.revision});return;
 }
 localStorage.setItem('s5-synthetic-storage','kept');sync('loop-start');
 while(true) {} // A real stuck WebContent thread, not an unresolved Promise.
}

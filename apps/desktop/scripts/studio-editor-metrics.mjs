/** Pure acceptance math. Raw samples and machine reports belong outside Git. */
export function distribution(values) {
  const sorted=values.filter(Number.isFinite).sort((a,b)=>a-b);
  const at=p=>sorted.length?sorted[Math.max(0,Math.ceil(sorted.length*p)-1)]:null;
  return {count:sorted.length,p50:at(.5),p95:at(.95),p99:at(.99),max:sorted.at(-1)??null};
}
/** Correlate input with the actual native image used by a completed composite.
 * Renderer timestamps share one clock. Host/IPC durations are separate bounds. */
export function inputLatencyMetrics(presentations, inputs) {
  const latencies = [], stages = [];
  for (const input of inputs) {
    if (input.kind !== 'pointer' && input.kind !== 'time') continue;
    for (const frame of presentations) {
      if (!Number.isFinite(frame.pixelAt) || frame.at < input.at) continue;
      const job = frame.jobs.find(j => !j.slot && j.inputAt >= input.at
        && (input.kind !== 'time' || (j.native?.host?.time ?? j.time) >= input.time - 1e-7));
      if (!job) continue;
      latencies.push(frame.pixelAt - input.at);
      const t = job.native?.timestamps;
      if (t) stages.push({
        inputToPatch: job.inputAt - input.at,
        patchToRequest: t.queuedAt - job.inputAt,
        queue: t.startedAt - t.queuedAt,
        upload: t.uploadedAt - t.startedAt,
        ipcHostSurface: t.replyAt - t.sentAt,
        host: job.native.host?.timings?.hostMs,
        surfaceToReady: t.readyAt - (t.textureAt ?? t.replyAt),
        readyToComposite: frame.at - t.readyAt,
        compositeToFence: frame.pixelAt - frame.at
      });
      break;
    }
  }
  return {
    inputToPixelMs: distribution(latencies), matchedInputs: latencies.length, totalInputs: inputs.length,
    pipelineMs: Object.fromEntries(Object.keys(stages[0] ?? {}).map(key => [key, distribution(stages.map(s => s[key]))]))
  };
}
/** Include a task that began just before the measured action in the same turn. */
export function overlappingTasks(tasks,start,end) {
  return tasks.filter(task=>task.at+task.ms>start&&task.at<end);
}
export function presentationMetrics(presentations,start,end,hz=60) {
  const rows=presentations.filter(p=>p.at>=start&&p.at<=end);
  // A compositor repaint of the same native image is not a new Studio frame.
  const unique=rows.filter((p,i)=>i===0||p.signature!==rows[i-1].signature);
  const intervals=unique.slice(1).map((p,i)=>p.at-unique[i].at);
  const seconds=(end-start)/1000,expected=Math.floor(seconds*Math.min(60,hz));
  return {seconds,presentations:unique.length,fps:unique.length/seconds,expected,
    droppedFrames:Math.max(0,expected-unique.length),
    beyond33ms:intervals.filter(v=>v>33.3).length,
    beyond33Percent:intervals.length?100*intervals.filter(v=>v>33.3).length/intervals.length:100,
    frameMs:distribution(intervals),
    pass:unique.length/seconds>=Math.min(60,hz)*.998&&intervals.length>0&&distribution(intervals).p95<=16.7&&distribution(intervals).p99<=33.3&&Math.max(...intervals)<100&&intervals.filter(v=>v>33.3).length/intervals.length<=.01};
}
export function memoryTrend(samples) {
  if(samples.length<2)return {growthPercent:null,slopeMiBPerMinute:null,pass:false};
  const mean=rows=>rows.reduce((n,r)=>n+r.rssMiB,0)/rows.length;
  const count=Math.min(3,Math.floor(samples.length/2)),baseline=mean(samples.slice(0,count)),retained=mean(samples.slice(-count));
  const points=samples.map(s=>[(s.at-samples[0].at)/60000,s.rssMiB]);
  const x=points.reduce((n,p)=>n+p[0],0)/points.length,y=mean(samples);
  const denominator=points.reduce((n,p)=>n+(p[0]-x)**2,0);
  const slope=denominator?points.reduce((n,p)=>n+(p[0]-x)*(p[1]-y),0)/denominator:0;
  const growthPercent=100*(retained-baseline)/baseline;
  return {baselineMiB:baseline,retainedMiB:retained,growthPercent,slopeMiBPerMinute:slope,pass:growthPercent<=10&&slope<=Math.max(1,baseline*.001)};
}

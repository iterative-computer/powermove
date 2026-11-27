// Actual tagged media-tool orchestration, with synthetic decoder/process seams.
// This proves the MCP boundary shape, not Metal/FFmpeg pixels or macOS hosting.
import { tag, evaluate } from './main-oracle.mjs';
import fs from 'node:fs';
import path from 'node:path';
const input=JSON.parse(fs.readFileSync(0,'utf8'));
const images=JSON.parse(fs.readFileSync(new URL('../fixtures/service-images.json',import.meta.url),'utf8'));
const {formatClock}=evaluate(tag('apps/desktop/src/shared/media-tools.ts'),['formatClock']);
const names=['FRAME_QUALITY','analysisArgs','compositionToSource','displaySize','fitBudget','frameArgs','parseLoudness','parseProbe','parseSilences','pickInformativeTimes','planSheet','resolveSampleTimes','round','sourceToComposition','tileArgs','triageScanArgs','waveformArgs'];
const ff=evaluate(tag('apps/desktop/src/main/agent-tools/media-ffmpeg.ts'),names,{formatClock});
const source={path:'/synthetic/footage.mp4',origin:'source',asset:{id:'a',name:'Synthetic footage',kind:'video',duration:30,hasAudio:true,proxy:false},...(input.layer?{layer:input.layer}:{})};
const module=tag('apps/desktop/src/main/agent-tools/media-tools.ts').replace(/^import[\s\S]*?;\n/gm,'').replace('constructor(private readonly options: AgentMediaToolsOptions) {}','constructor(options: AgentMediaToolsOptions) { this.options=options; }');
const {AgentMediaTools}=evaluate(module,['AgentMediaTools'],{...ff,formatClock,path,Buffer,Date,AbortController,setTimeout,clearTimeout,writeFile:async()=>{},readFile:async filename=>Buffer.from(filename.endsWith('.png')?images.png:images.jpeg,'base64')});
const tools=new AgentMediaTools({fontFile:null});
tools.probe=async()=>({format:'mov/mp4',duration:30,start:0,bitrateKbps:114,video:[{index:0,codec:'h264',pixelFormat:'yuv420p',width:640,height:360,fps:30,alpha:false}],audio:[{index:1,codec:'aac',sampleRate:48000,channels:2}],otherStreams:0});
tools.frame=async()=>{};
tools.withTemp=work=>work('/synthetic/no-files');
tools.ffmpeg=async()=>({code:0,stdout:Buffer.alloc(32*32*3*4),stderr:'[silencedetect] silence_start: 2\n[silencedetect] silence_end: 3.5 | silence_duration: 1.5\n'});
const context={resolve:async()=>source,composition:async()=>({width:640,height:360,fps:30,duration:20,workArea:[0,20]}),renderFrames:async times=>times.slice(0,input.completed??times.length).map(()=>({mimeType:'image/png',data:Buffer.from(images.png,'base64')}))};
const content=await tools.call(input.tool,input.arguments??{},context);
const result={value:JSON.parse(content[0].text),images:content.slice(1).map(i=>({data:Buffer.from(i.data).toString('base64'),mimeType:i.mimeType}))};
process.stdout.write(JSON.stringify(result));

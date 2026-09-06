'use client';

import { useEffect, useRef, useState } from 'react';
import { ArrowUp, ArrowUpRight, Check, Pause, Play, RotateCcw, Sparkles } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';

const revisions = [
  { label: 'The first idea', prompt: 'Create a cinematic title with a warm, glowing atmosphere.', reply: 'A little atmosphere. A bold title. A place to start.', image: '/editor-0.png' },
  { label: 'A little bolder', prompt: 'Change the title to “Make it bold.”', reply: 'New words. Same composition. Keep the idea moving.', image: '/editor-1.png' },
  { label: 'A new direction', prompt: 'Change the mood. Make the atmosphere violet.', reply: 'A cooler palette and a new title. The layers stay yours.', image: '/editor-2.png' },
];

function Shader({ revision, paused }: { revision: number; paused: boolean }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const state = useRef({ revision, paused });
  state.current = { revision, paused };
  useEffect(() => {
    const canvas = ref.current!;
    const gl = canvas.getContext('webgl', { alpha: true, antialias: false, powerPreference: 'low-power' });
    if (!gl) return;
    const compile = (type: number, source: string) => {
      const shader = gl.createShader(type)!; gl.shaderSource(shader, source); gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) { gl.deleteShader(shader); return null; }
      return shader;
    };
    const vertex = compile(gl.VERTEX_SHADER, 'attribute vec2 p;void main(){gl_Position=vec4(p,0.,1.);}');
    const fragment = compile(gl.FRAGMENT_SHADER, `precision mediump float;
      uniform vec2 resolution; uniform float time; uniform float mood;
      void main(){
        vec2 uv=gl_FragCoord.xy/resolution;vec2 p=(uv-.5)*vec2(resolution.x/resolution.y,1.);
        float t=time*.12;float sum=0.;float haze=0.;
        for(int i=0;i<24;i++){
          float f=float(i)/24.;
          float wave=sin(p.x*2.4+t+f*3.)*.17+sin(p.x*4.4-t*.7+f*2.)*.055;
          float y=wave+(f-.5)*.24-.15;
          float d=abs(p.y-y);sum+=.0017/(d+.003);haze+=.006/(d+.09);
        }
        vec3 a=mix(vec3(1.,.24,.045),vec3(.4,.22,1.),mood);
        vec3 b=mix(vec3(1.,.65,.28),vec3(.95,.3,.72),mood);
        vec3 col=mix(a,b,smoothstep(-.7,.8,p.x))*sum*.26+a*haze*.065;
        float edge=smoothstep(0.,.2,uv.x)*smoothstep(1.,.78,uv.x);
        float grain=fract(sin(dot(gl_FragCoord.xy,vec2(12.9898,78.233)))*43758.5453);
        gl_FragColor=vec4(col*edge+grain*.013,1.);
      }`);
    if (!vertex || !fragment) return;
    const program = gl.createProgram()!;gl.attachShader(program,vertex);gl.attachShader(program,fragment);gl.linkProgram(program);
    if (!gl.getProgramParameter(program,gl.LINK_STATUS)) return;
    gl.useProgram(program);
    const buffer=gl.createBuffer();gl.bindBuffer(gl.ARRAY_BUFFER,buffer);gl.bufferData(gl.ARRAY_BUFFER,new Float32Array([-1,-1,1,-1,-1,1,-1,1,1,-1,1,1]),gl.STATIC_DRAW);
    const pos=gl.getAttribLocation(program,'p');gl.enableVertexAttribArray(pos);gl.vertexAttribPointer(pos,2,gl.FLOAT,false,0,0);
    const res=gl.getUniformLocation(program,'resolution'),time=gl.getUniformLocation(program,'time'),mood=gl.getUniformLocation(program,'mood');
    const motion=matchMedia('(prefers-reduced-motion: reduce)');let frame=0,t=0,last=0,currentMood=0,visible=true;
    const observer=new IntersectionObserver(([entry])=>{visible=entry.isIntersecting;});observer.observe(canvas);
    const resize=()=>{const dpr=Math.min(devicePixelRatio,1.5);canvas.width=Math.round(canvas.clientWidth*dpr);canvas.height=Math.round(canvas.clientHeight*dpr);gl.viewport(0,0,canvas.width,canvas.height);};
    const size=new ResizeObserver(resize);size.observe(canvas);resize();
    const draw=(now:number)=>{const dt=last?Math.min((now-last)/1000,.05):0;last=now;
      if(visible&&!document.hidden){if(!state.current.paused&&!motion.matches)t+=dt;currentMood+=(Number(state.current.revision===2)-currentMood)*.06;gl.uniform2f(res,canvas.width,canvas.height);gl.uniform1f(time,t);gl.uniform1f(mood,currentMood);gl.drawArrays(gl.TRIANGLES,0,6);}
      frame=requestAnimationFrame(draw);
    };frame=requestAnimationFrame(draw);
    return()=>{cancelAnimationFrame(frame);observer.disconnect();size.disconnect();gl.deleteBuffer(buffer);gl.deleteProgram(program);gl.deleteShader(vertex);gl.deleteShader(fragment);};
  },[]);
  return <canvas ref={ref} className="shader" aria-hidden="true" />;
}

export default function Home() {
  const [revision,setRevision]=useState(0);
  const [prompt,setPrompt]=useState(revisions[1].prompt);
  const [paused,setPaused]=useState(false);
  const [message,setMessage]=useState('');
  const current=revisions[revision];
  function choose(index:number){setRevision(index);setPrompt(revisions[(index+1)%3].prompt);setMessage('');}
  function submit(event:React.FormEvent){event.preventDefault();const text=prompt.trim().toLowerCase();
    if(/violet|purple|mood|cool/.test(text))choose(2);
    else if(/bold|title|words/.test(text))choose(1);
    else if(/warm|glow|cinematic|atmosphere|start/.test(text))choose(0);
    else setMessage('This is a guided preview. Try one of the example prompts below.');
  }
  return <main>
    <Shader revision={revision} paused={paused}/>
    <header className="site-header"><a href="#" className="wordmark" aria-label="Powermove home"><span className="brand-mark" aria-hidden="true">↗</span>powermove</a><span className="availability"><span/>In development</span></header>
    <section className="hero" aria-labelledby="headline">
      <p className="eyebrow"><span/> THE NEXT MOVE IS YOURS</p>
      <h1 id="headline">From a thought.<br/><span>To a powermove.</span></h1>
      <p className="intro">An AI-native motion editor for the ideas that won’t sit still.<br className="desktop-break"/> Start with a prompt. Shape every detail. Make it yours.</p>
      <a className="explore" href="#playground">See an idea take shape <span>↓</span></a>
    </section>
    <section id="playground" className="playground" aria-label="Guided editor preview">
      <div className="demo-topline"><span><span className="live-dot"/> A WORK IN MOTION</span><Button variant="ghost" className="motion-control" onClick={()=>setPaused(!paused)} aria-label={paused?'Resume background animation':'Pause background animation'}>{paused?<Play/>:<Pause/>}<span>{paused?'Resume motion':'Pause motion'}</span></Button></div>
      <div className="editor-frame">
        <div className="window-bar"><span className="traffic" aria-hidden="true"><i/><i/><i/></span><span>Powermove <span className="muted">/</span> An idea in progress</span><span className="window-version">EARLY LOOK</span></div>
        <div className="editor-images">{revisions.map((item,index)=><img key={item.image} src={item.image} width="1600" height="1000" className={revision===index?'editor-image active':'editor-image'} alt={revision===index?`Powermove editor — ${item.label}. Editable text, atmosphere layer, properties, and keyframes.`:''} aria-hidden={revision!==index} fetchPriority={index===0?'high':'auto'}/>)}</div>
        <div className="editor-caption"><span><Check size={14}/> Real editor. Real layers. Room to keep going.</span><span>0{revision+1} / 03</span></div>
      </div>
      <div className="prompt-card">
        <div className="prompt-heading"><span className="assistant-icon"><Sparkles size={16}/></span><span>One idea. A few good turns.</span><span className="demo-label">GUIDED DEMO</span></div>
        <p className="assistant-reply" aria-live="polite">{current.reply}</p>
        <form onSubmit={submit} className="prompt-form"><label htmlFor="prompt" className="sr-only">Try an example direction</label><Textarea id="prompt" rows={2} maxLength={500} value={prompt} onChange={e=>setPrompt(e.target.value)} placeholder="Give your idea a new direction…"/><Button type="submit" className="send" disabled={!prompt.trim()} aria-label="Apply demo prompt"><ArrowUp size={20}/></Button></form>
        {message&&<p className="form-message" role="status">{message}</p>}
        <div className="suggestions" aria-label="Example prompts"><span>Try a turn</span><Button variant="ghost" onClick={()=>choose(1)}>Make it bold <ArrowUpRight/></Button><Button variant="ghost" onClick={()=>choose(2)}>Go violet <ArrowUpRight/></Button><Button variant="ghost" onClick={()=>choose(0)} aria-label="Reset demo"><RotateCcw/>Start over</Button></div>
      </div>
      <div className="revision-track" aria-label="Composition revisions">{revisions.map((item,index)=><Button variant="ghost" key={item.label} onClick={()=>choose(index)} aria-pressed={revision===index} className={revision===index?'revision selected':'revision'}><span className="revision-number">0{index+1}</span><span>{item.label}</span><span className="revision-line"/></Button>)}</div>
      <p className="preview-note">A small preview of what’s taking shape. Example prompts replay prepared editor variations.</p>
    </section>
    <section className="closing"><p>Prompt the possibility.<br/><span>Own the last detail.</span></p><span>We’re building Powermove.<br/>More soon.</span></section>
    <footer><a href="#" className="wordmark"><span className="brand-mark" aria-hidden="true">↗</span>powermove</a><span>A new motion from Motioner.</span><span>© {new Date().getFullYear()} Powermove</span></footer>
  </main>;
}

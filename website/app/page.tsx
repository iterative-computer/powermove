'use client';

import { ArrowDown, ArrowUpRight, Layers3, SlidersHorizontal, Sparkles } from 'lucide-react';
import { HeroAnimation } from './hero-animation';

const features = [
  { icon: Sparkles, title: 'Create with AI', text: 'Work with your connected agent to turn an idea into an editable composition.' },
  { icon: SlidersHorizontal, title: 'Shape every frame', text: 'Refine the timing, adjust a curve, and get every detail exactly where you want it.' },
  { icon: Layers3, title: 'Keep full control', text: 'Real layers, properties, and keyframes. Everything stays yours to change.' },
];

function Logo() {
  return <a className="logo" href="#top" aria-label="Powermove home"><img src="/powermove-logo.svg" width={28} height={24} alt=""/><span>Powermove</span></a>;
}

export default function Home() {
  return <main id="top">
    <header className="navigation"><Logo/><nav aria-label="Main navigation"><a href="#editor">Editor</a><a href="#features">Features</a></nav><a className="button" href="#preview">Explore Powermove <ArrowDown size={13}/></a></header>
    <section className="hero" aria-labelledby="headline">
      <h1 id="headline" className="sr-only">Make a Powermove.</h1>
      <HeroAnimation/>
      <div className="hero-intro"><p>Powermove is the versatile motion design software that lets you shape its feature set.</p><a className="button primary" href="#editor">Explore the editor <ArrowDown size={14}/></a></div>
    </section>
    <section className="editor-section" id="editor" aria-labelledby="editor-title">
      <div className="section-heading"><h2 id="editor-title">Your ideas. Your workspace.</h2><p>Everything you need to go from the first prompt to the final keyframe.</p></div>
      <figure className="editor-window" id="preview"><img src="/editor-0.png" alt="Powermove editor with a cinematic title, layer stack, property inspector, AI agent, and keyframe timeline." width={1600} height={1000} loading="lazy"/></figure>
      <div className="features" id="features">{features.map(({icon: Icon, title, text})=><article key={title}><div className="feature-icon"><Icon size={18} strokeWidth={1.5}/></div><h3>{title}</h3><p>{text}</p></article>)}</div>
    </section>
    <section className="closing"><div><h2>Made for your next idea.</h2><p>Powermove is in development. More soon.</p></div><a className="button" href="#top">Back to the animation <ArrowUpRight size={14}/></a></section>
    <footer><Logo/><span>© {new Date().getFullYear()} Powermove</span></footer>
  </main>;
}

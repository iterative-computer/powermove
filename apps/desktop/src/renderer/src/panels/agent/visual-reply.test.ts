// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';
import { prepareVisualReply, visualDocument } from './visual-reply';
import { AgentThreads } from './threads';
describe('visual reply validation and persistence', () => {
  it.each(['<script src="app://powermove/host/shim.js"></script>','<img src="file:///etc/passwd">','<img src="/tmp/shot.png">','<img src="https://example.com/a.png">','<iframe srcdoc="hi"></iframe>','<template><img src="/tmp/private.png"></template>','<base href="app://powermove">','<meta http-equiv="refresh" content="0;url=https://example.com">','<form action="https://example.com">x</form>','<a href="javascript:alert(1)">x</a>','<svg><image href="https://example.com"/></svg>'])('rejects ambient resources: %s', html => {
    expect(() => prepareVisualReply({ title:'x',html })).toThrow();
  });
  it('validates embedded raster signatures and per-image encoded size', () => {
    expect(() => prepareVisualReply({title:'x',html:'<img src="data:image/png;base64,c2VjcmV0">'})).toThrow('raster format');
    const valid='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aO+sAAAAASUVORK5CYII=';
    expect(prepareVisualReply({title:'x',html:`<img src="${valid}">`}).html).toContain(valid);
    expect(() => prepareVisualReply({title:'x',html:'<img src="data:image/png;base64,'+'A'.repeat(262144)+'">'})).toThrow('192 KiB');
  });
  it('retains inline scripts, SVG, accessible controls and small embedded images', () => {
    const v = prepareVisualReply({title:'Easing',html:'<style>svg{color:var(--accent)}</style><svg viewBox="0 0 10 10"><path d="M0 0L10 10"/></svg><button aria-label="Replay">Replay</button><script>let n=1;</script>'});
    expect(v.html).toContain('<script>let n=1;</script>');
    const doc = visualDocument(v.html,'nonce');
    expect(doc.indexOf('Content-Security-Policy')).toBeLessThan(doc.indexOf('let n=1'));
    expect(doc).toContain('powermove-visual-theme');
  });
  it('reopens complete immutable page bytes with their message and removes association on deletion', () => {
    const data = new Map<string, unknown>();
    const store = { get:(k:string,f:unknown) => data.get(k) ?? f, set:(k:string,v:unknown) => {data.set(k,structuredClone(v));return true;} };
    const archive = new AgentThreads(store,()=>'thread-one'); archive.load('project');
    const visual = { version:1 as const,id:'visual-one',runId:'run-one',...prepareVisualReply({title:'Storyboard',html:'<h1>Arrival</h1>'}) };
    archive.active.conversation.push({role:'assistant',visualReplies:[visual]}); expect(archive.save()).toBe(true);
    const reopened = new AgentThreads(store,()=>'thread-two');reopened.load('project');
    expect(reopened.active.conversation[0]?.visualReplies?.[0]).toEqual(visual);
    reopened.remove('thread-one'); reopened.save();
    expect(JSON.stringify(data.get('agentThreads.project'))).not.toContain('Arrival');
    store.set = () => false;
    reopened.active.composerDraft = 'storage failure'; expect(reopened.save()).toBe(false);
  });
});

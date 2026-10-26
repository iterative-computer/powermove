import { describe, expect, it } from 'vitest';
import { isVisualReplyUrl, readVisualReply, visualReplyEvent, visualReplyInput, VISUAL_REPLY_CSP } from './visual-reply';
describe('visual reply trust boundary', () => {
  it('bounds bytes, dimensions, titles and closed input', () => {
    expect(visualReplyInput({ title: 'Chart', html: '<svg></svg>' }).height).toBe(360);
    for (const v of [{ title: '', html: 'a' }, { title:'a', html:'😀'.repeat(100000) }, { title:'a', html:'b', height: NaN }, { title:'a', html:'b', height:901 }, { title:'a', html:'b', path:'/etc/passwd' }]) expect(() => visualReplyInput(v)).toThrow();
    expect(readVisualReply({version:1,id:'../bad',runId:'run',title:'a',html:'b',height:300})).toBeNull();
  });
  it('accepts only messages from the mounted opaque frame, with exact token and schema', () => {
    const source = {} as Window;
    const event = {origin:'null',source,data:{channel:'powermove-visual',token:'nonce',type:'size',height:500}};
    expect(visualReplyEvent(event,source,'nonce')).toEqual({type:'size',height:500});
    expect(visualReplyEvent({...event,origin:'app://powermove'},source,'nonce')).toBeNull();
    expect(visualReplyEvent({...event,source:{} as Window},source,'nonce')).toBeNull();
    expect(visualReplyEvent(event,source,'stale')).toBeNull();
    for (const data of [{channel:'powermove-visual',token:'nonce',type:'ready',tool:'run_outside_sandbox'},{...event.data,height:Infinity},{...event.data,type:'invoke',tool:'run_outside_sandbox'}]) expect(visualReplyEvent({...event,data},source,'nonce')).toBeNull();
  });
  it('restricts page capabilities and identifies a committed page independently of its name', () => {
    for (const d of ["connect-src 'none'","frame-src 'none'","worker-src 'none'","form-action 'none'","base-uri 'none'"]) expect(VISUAL_REPLY_CSP).toContain(d);
    expect(VISUAL_REPLY_CSP).not.toContain('unsafe-eval');
    expect(isVisualReplyUrl('app://powermove/visual-reply.html#changed')).toBe(true);
    expect(isVisualReplyUrl('app://powermove/index.html')).toBe(false);
    expect(isVisualReplyUrl('https://evil.example/visual-reply.html')).toBe(false);
  });
});

// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';
import { publishVisualReply } from './publish-visual-reply';
import { AgentThreads, threadMessageText } from './threads';
describe('run-scoped visual publication', () => {
  function setup() {
    let serial = 0;
    const uid = (prefix='thread-') => prefix + ++serial;
    const data = new Map<string,unknown>();
    const set = vi.fn((k:string,v:unknown) => {data.set(k,structuredClone(v));return true;});
    const store = {get:(k:string,f:unknown) => data.get(k) ?? f,set};
    const archive = new AgentThreads(store,uid);archive.load('project');
    const originalThread = archive.active;
    const session = {codexRequestId:'live-run',projectId:'project',threadId:originalThread.id,conversation:originalThread.conversation};
    archive.create(); // Publishing must not target the newly selected thread.
    // A blank original is reused; explicitly add another thread for this check.
    archive.threads.push({...originalThread,id:'visible-other',conversation:[]}); archive.activeId='visible-other';
    const changed = vi.fn();
    const options = {runId:'live-run',value:{title:'Palette',html:'<h1>Colors</h1>'},sessions:[session],busy:()=>true,archive:()=>archive,uid,changed};
    return {archive,session,options,set,store};
  }
  it('binds the artifact to its caller and survives reopening with follow-up context', () => {
    const {archive,session,options,store} = setup();
    const result=publishVisualReply(options);
    expect(result).toMatchObject({saved:true,threadId:session.threadId});
    expect(archive.active.conversation).toEqual([]);
    expect(threadMessageText(session.conversation[0]!)).toContain('Visual reply: Palette');
    const reopened=new AgentThreads(store,()=> 'fallback');reopened.load('project');
    expect(reopened.threads.find(t=>t.id===session.threadId)?.conversation[0]?.visualReplies?.[0]?.id).toBe(result.id);
  });
  it('rejects inactive/unknown runs, removed threads and storage failures without false success', () => {
    const {archive,session,options,set} = setup();
    expect(()=>publishVisualReply({...options,runId:'forged'})).toThrow('no longer active');
    expect(()=>publishVisualReply({...options,busy:()=>false})).toThrow('no longer active');
    expect(()=>publishVisualReply({...options,archive:()=>undefined})).toThrow('no longer open');
    set.mockReturnValue(false);
    expect(()=>publishVisualReply(options)).toThrow('could not be saved');
    expect(session.conversation).toEqual([]); expect(options.changed).not.toHaveBeenCalled();
    archive.remove(session.threadId);
    expect(()=>publishVisualReply(options)).toThrow('removed');
  });
  it('caps output per run and rejects forbidden input before changing the archive', () => {
    const {session,options} = setup();
    expect(()=>publishVisualReply({...options,value:{title:'x',html:'<img src="/tmp/private.png">'}})).toThrow();
    expect(session.conversation).toHaveLength(0);
    for(let i=0;i<4;i++) publishVisualReply(options);
    expect(()=>publishVisualReply(options)).toThrow('four visual replies');
    expect(session.conversation).toHaveLength(1);
    expect(session.conversation[0]!.visualReplies).toHaveLength(4);
  });
  it('attaches one or multiple pages to prose without replacing it, and rolls back failed appends', () => {
    const {session,options,set,store} = setup();
    session.conversation.push({role:'assistant',text:'Here is the reasoning, with a visual attached.'});
    publishVisualReply(options); publishVisualReply(options);
    expect(session.conversation).toHaveLength(1);
    expect(session.conversation[0]!.text).toBe('Here is the reasoning, with a visual attached.');
    expect(session.conversation[0]!.visualReplies).toHaveLength(2);
    set.mockReturnValue(false);
    expect(()=>publishVisualReply(options)).toThrow('could not be saved');
    expect(session.conversation[0]!.visualReplies).toHaveLength(2);
    const reopened=new AgentThreads(store,()=> 'fallback');reopened.load('project');
    expect(reopened.threads.find(t=>t.id===session.threadId)?.conversation[0]?.visualReplies).toHaveLength(2);
  });
});

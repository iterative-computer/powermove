import {describe,expect,it} from 'vitest';
import {overlayFrameKey,hasOverlayFrameKey} from './overlay-frame';

describe('overlay frame identity',()=>{
  it('rejects a later color frame even when revision and view sequence are unchanged',()=>{
    const chrome={revision:3,viewSeq:8,seq:17};
    expect(overlayFrameKey(chrome)).toBe('3:8:17');
    const capturedKey=overlayFrameKey(chrome);
    expect(hasOverlayFrameKey({...chrome},capturedKey)).toBe(true);
    expect(hasOverlayFrameKey({...chrome,seq:18},capturedKey)).toBe(false);
    chrome.seq=18;
    expect(hasOverlayFrameKey(chrome,capturedKey)).toBe(false);
    expect(hasOverlayFrameKey(null,capturedKey)).toBe(false);
  });
});

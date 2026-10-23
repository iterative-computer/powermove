/** The presented color frame and its chrome planes share one frame sequence. */
export type OverlayFrameIdentity = {revision:number;viewSeq:number;seq:number};

export function overlayFrameKey(frame:OverlayFrameIdentity):string {
  return `${frame.revision}:${frame.viewSeq}:${frame.seq}`;
}

export function hasOverlayFrameKey(current:OverlayFrameIdentity|null|undefined,expectedKey:string|null):boolean {
  return !!current&&expectedKey!==null&&overlayFrameKey(current)===expectedKey;
}

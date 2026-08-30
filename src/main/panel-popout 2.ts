const PANEL_FRAME = /^pm-panel-[A-Za-z0-9_-]{1,80}$/;

export function isPanelPopoutRequest(
  url: string,
  frameName: string,
  openerAllowed: boolean
): boolean {
  return openerAllowed && url === 'about:blank' && PANEL_FRAME.test(frameName);
}

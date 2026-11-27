type N = any;
export type SelectionState = {
  anchor: number | null;
  anchor_offset: number;
  focus: number | null;
  focus_offset: number;
};
/** Stable realm-local text selection identity. Geometry remains asynchronous. */
export class RemoteSelection {
  anchorNode: N = null;
  anchorOffset = 0;
  focusNode: N = null;
  focusOffset = 0;
  constructor(private changed: (selection: RemoteSelection) => void) {}
  get isCollapsed() {
    return this.anchorNode === this.focusNode && this.anchorOffset === this.focusOffset;
  }
  get rangeCount() {
    return this.anchorNode ? 1 : 0;
  }
  get type() {
    return !this.anchorNode ? 'None' : this.isCollapsed ? 'Caret' : 'Range';
  }
  setBaseAndExtent(anchor: N, anchorOffset: number, focus: N, focusOffset: number) {
    checkOffset(anchor, anchorOffset);
    checkOffset(focus, focusOffset);
    this.anchorNode = anchor;
    this.anchorOffset = anchorOffset;
    this.focusNode = focus;
    this.focusOffset = focusOffset;
    this.changed(this);
  }
  collapse(node: N, offset = 0) {
    if (!node) this.removeAllRanges();
    else this.setBaseAndExtent(node, offset, node, offset);
  }
  setPosition(node: N, offset = 0) {
    this.collapse(node, offset);
  }
  extend(node: N, offset = 0) {
    this.setBaseAndExtent(this.anchorNode, this.anchorOffset, node, offset);
  }
  removeAllRanges() {
    this.anchorNode = this.focusNode = null;
    this.anchorOffset = this.focusOffset = 0;
    this.changed(this);
  }
  empty() {
    this.removeAllRanges();
  }
  addRange(range: RemoteRange) {
    this.setBaseAndExtent(
      range.startContainer,
      range.startOffset,
      range.endContainer,
      range.endOffset
    );
  }
  getRangeAt(index: number) {
    if (index !== 0 || !this.rangeCount) throw new RangeError('IndexSizeError');
    const range = new RemoteRange(this.anchorNode.ownerDocument);
    const [start, startOffset, end, endOffset] = ordered(
      this.anchorNode,
      this.anchorOffset,
      this.focusNode,
      this.focusOffset
    );
    range.setStart(start, startOffset);
    range.setEnd(end, endOffset);
    return range;
  }
  selectAllChildren(node: N) {
    this.setBaseAndExtent(node, 0, node, node.childNodes.length);
  }
  toString() {
    return this.rangeCount ? this.getRangeAt(0).toString() : '';
  }
}
function checkOffset(node: N, offset: number) {
  if (
    !node ||
    !Number.isInteger(offset) ||
    offset < 0 ||
    offset >
      (node.nodeType === 3 || node.nodeType === 8 ? node.data.length : node.childNodes.length)
  )
    throw new RangeError('IndexSizeError');
}
function textPosition(node: N, offset: number): number {
  const root = node.ownerDocument;
  let position = 0,
    found = false;
  function visit(current: N) {
    if (current === node) {
      if (current.nodeType === 3) position += offset;
      else
        for (const child of [...current.childNodes].slice(0, offset))
          position += child.textContent?.length ?? 0;
      found = true;
      return;
    }
    if (current.nodeType === 3) {
      position += current.data.length;
      return;
    }
    for (const child of current.childNodes) {
      visit(child);
      if (found) break;
    }
  }
  visit(root);
  return position;
}
function ordered(a: N, ao: number, b: N, bo: number): [N, number, N, number] {
  return textPosition(a, ao) <= textPosition(b, bo) ? [a, ao, b, bo] : [b, bo, a, ao];
}
export class RemoteRange {
  startContainer: N;
  endContainer: N;
  startOffset = 0;
  endOffset = 0;
  constructor(private document: N) {
    this.startContainer = this.endContainer = document;
  }
  get collapsed() {
    return this.startContainer === this.endContainer && this.startOffset === this.endOffset;
  }
  setStart(node: N, offset: number) {
    checkOffset(node, offset);
    this.startContainer = node;
    this.startOffset = offset;
  }
  setEnd(node: N, offset: number) {
    checkOffset(node, offset);
    this.endContainer = node;
    this.endOffset = offset;
  }
  selectNodeContents(node: N) {
    this.setStart(node, 0);
    this.setEnd(
      node,
      node.nodeType === 3 || node.nodeType === 8 ? node.data.length : node.childNodes.length
    );
  }
  selectNode(node: N) {
    const parent = node.parentNode;
    const index = [...parent.childNodes].indexOf(node);
    this.setStart(parent, index);
    this.setEnd(parent, index + 1);
  }
  collapse(toStart = false) {
    if (toStart) this.setEnd(this.startContainer, this.startOffset);
    else this.setStart(this.endContainer, this.endOffset);
  }
  cloneRange() {
    const range = new RemoteRange(this.document);
    range.setStart(this.startContainer, this.startOffset);
    range.setEnd(this.endContainer, this.endOffset);
    return range;
  }
  createContextualFragment(html: string) {
    const template = this.document.createElement('template');
    template.innerHTML = html;
    return template.content;
  }
  toString() {
    const start = textPosition(this.startContainer, this.startOffset),
      end = textPosition(this.endContainer, this.endOffset);
    function text(node: N): string {
      return node.nodeType === 3 ? node.data : [...node.childNodes].map(text).join('');
    }
    return text(this.document).slice(start, end);
  }
  detach() {}
}

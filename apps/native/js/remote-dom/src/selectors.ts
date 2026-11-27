import { is, selectAll } from 'css-select';
import { parse, type Selector } from 'css-what';
type N = any;
/** Property-sensitive pseudos must read live form/focus state, not attributes. */
const adapter: any = {
  isTag: (n: N) => n.nodeType === 1,
  getChildren: (n: N) => [...(n.childNodes ?? [])],
  getParent: (n: N) => n.parentNode,
  getSiblings: (n: N) => (n.parentNode ? [...n.parentNode.childNodes] : [n]),
  getAttributeValue: (n: N, name: string) => n.getAttribute(name) ?? undefined,
  hasAttrib: (n: N, name: string) => n.hasAttribute(name),
  getName: (n: N) => n.localName,
  getText: (n: N) => n.textContent ?? '',
  removeSubsets: (nodes: N[]) =>
    nodes.filter(
      (node, i) =>
        nodes.indexOf(node) === i && !nodes.some((other) => other !== node && other.contains(node))
    ),
  existsOne(test: (node: N) => boolean, nodes: N[]): boolean {
    return nodes.some(
      (node) => node.nodeType === 1 && (test(node) || adapter.existsOne(test, [...node.childNodes]))
    );
  },
  findAll(test: (node: N) => boolean, nodes: N[]): N[] {
    return nodes.flatMap((node) =>
      node.nodeType === 1
        ? [...(test(node) ? [node] : []), ...adapter.findAll(test, [...node.childNodes])]
        : []
    );
  },
  findOne(test: (node: N) => boolean, nodes: N[]): N {
    for (const node of nodes) {
      if (node.nodeType !== 1) continue;
      if (test(node)) return node;
      const found = adapter.findOne(test, [...node.childNodes]);
      if (found) return found;
    }
    return null;
  }
};
const pseudos = {
  'pm-checked': (node: N) =>
    node.localName === 'option'
      ? node.selected
      : ['checkbox', 'radio'].includes(node.type) && node.checked,
  'pm-selected': (node: N) => node.localName === 'option' && node.selected,
  'pm-focus': (node: N) => node.ownerDocument.activeElement === node,
  'pm-disabled': (node: N) => !!node.disabled,
  'pm-enabled': (node: N) =>
    ['input', 'select', 'textarea', 'button', 'option', 'optgroup', 'fieldset'].includes(
      node.localName
    ) && !node.disabled
};
export const dynamicSelector = /:(?:checked|selected|focus|disabled|enabled)\b/;
function tokens(selector: string): Selector[][] {
  const parsed = parse(selector);
  function visit(groups: Selector[][]) {
    for (const group of groups)
      for (const token of group) {
        if (token.type === 'pseudo') {
          if (['checked', 'selected', 'focus', 'disabled', 'enabled'].includes(token.name))
            token.name = 'pm-' + token.name;
          if (Array.isArray(token.data)) visit(token.data);
        }
      }
  }
  visit(parsed);
  return parsed;
}
export function query(node: N, selector: string): N[] {
  return selectAll(tokens(selector), [...node.childNodes], { adapter, pseudos, context: [node] });
}
export function matches(node: N, selector: string): boolean {
  return is(node, tokens(selector), { adapter, pseudos });
}

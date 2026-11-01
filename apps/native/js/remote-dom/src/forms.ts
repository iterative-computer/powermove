/** String-valued successful form controls. File capabilities are host work. */
type N = any;
export class RemoteFormData implements Iterable<[string, string | Blob]> {
  private pairs: [string, string | Blob][] = [];
  constructor(form?: N, submitter?: N) {
    if (!form) return;
    if (form.localName !== 'form') throw new TypeError('Expected form element');
    for (const control of form.querySelectorAll('input,select,textarea,button')) {
      if (!control.name || control.disabled || control.closest('fieldset[disabled]')) continue;
      if (
        (control.localName === 'button' ||
          ['button', 'submit', 'reset', 'image'].includes(control.type)) &&
        control !== submitter
      )
        continue;
      if (['checkbox', 'radio'].includes(control.type) && !control.checked) continue;
      if (control.type === 'file')
        throw new Error('File inputs require host-backed file capabilities');
      if (control.localName === 'select') {
        for (const option of control.options)
          if (option.selected && !option.disabled) this.append(control.name, option.value);
      } else this.append(control.name, control.value ?? '');
    }
  }
  append(name: string, value: string | Blob) {
    this.pairs.push([String(name), typeof value === 'string' ? value : value]);
  }
  delete(name: string) {
    this.pairs = this.pairs.filter((pair) => pair[0] !== String(name));
  }
  get(name: string) {
    return this.pairs.find((pair) => pair[0] === String(name))?.[1] ?? null;
  }
  getAll(name: string) {
    return this.pairs.filter((pair) => pair[0] === String(name)).map((pair) => pair[1]);
  }
  has(name: string) {
    return this.pairs.some((pair) => pair[0] === String(name));
  }
  set(name: string, value: string | Blob) {
    name = String(name);
    const index = this.pairs.findIndex((pair) => pair[0] === name);
    this.delete(name);
    this.pairs.splice(index < 0 ? this.pairs.length : index, 0, [name, value]);
  }
  *entries() {
    yield* this.pairs;
  }
  *keys() {
    for (const [key] of this.pairs) yield key;
  }
  *values() {
    for (const [, value] of this.pairs) yield value;
  }
  [Symbol.iterator]() {
    return this.entries();
  }
  forEach(
    callback: (value: string | Blob, key: string, parent: RemoteFormData) => void,
    thisArg?: unknown
  ) {
    for (const [key, value] of this.pairs) callback.call(thisArg, value, key, this);
  }
}

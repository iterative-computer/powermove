import {
  Component,
  createElement,
  type ComponentType,
  type FunctionComponent,
} from "preact";

/** Memoization without preact/compat's global DOM event/property rewrites. */
export function memo<P extends object>(
  render: FunctionComponent<P>,
  equal: (before: Readonly<P>, after: Readonly<P>) => boolean,
): ComponentType<P> {
  return class NativeMemo extends Component<P> {
    shouldComponentUpdate(next: P): boolean {
      return !equal(this.props, next);
    }
    render() {
      return createElement(render, this.props);
    }
  };
}

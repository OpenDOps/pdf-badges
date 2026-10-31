import {
  cloneElement,
  createElement,
  Component as ReactComponent,
  Fragment,
  isValidElement,
  type Attributes,
  type ComponentType,
  type ReactElement,
  type ReactNode,
  type Ref,
} from 'react';
import { createRoot, type Root } from 'react-dom/client';

export { Fragment };
export type { Attributes, ComponentType };

type DomRef = Ref<HTMLElement>;

/** The slice of Preact these components import. React apps alias `preact` here. */
export class Component<P = object, S = object> extends ReactComponent<P, S> {
  base: HTMLElement | null = null;

  constructor(props: P) {
    super(props);
    const userRender = this.render.bind(this);
    this.render = () => {
      const node = userRender();
      if (!isValidElement(node)) return node;
      const previous = (node as ReactElement & { ref?: DomRef }).ref;
      return cloneElement(node as ReactElement<{ ref?: DomRef }>, {
        ref: (el: HTMLElement | null) => {
          this.base = el;
          if (typeof previous === 'function') previous(el);
          else if (previous && typeof previous === 'object') previous.current = el;
        },
      });
    };
  }
}

export function h(type: unknown, props: Record<string, unknown> | null, ...children: ReactNode[]): ReactNode {
  const next = props ? { ...props } : null;
  if (next && 'ontouchstart' in next) {
    next.onTouchStart = next.ontouchstart;
    delete next.ontouchstart;
  }
  if (children.length > 0) return createElement(type as never, next, ...children);
  return createElement(type as never, next);
}

const roots = new WeakMap<Element, Root>();

export function render(vnode: ReactNode, parent: Element): void {
  let root = roots.get(parent);
  if (!root) {
    root = createRoot(parent);
    roots.set(parent, root);
  }
  root.render(vnode);
}

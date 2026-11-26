/* A component that is drawn again only when what it is given has changed (each prop compared with ===). The rows of
   the sidebar are these: a step taken by one chat repaints that chat's row and no other. */
import { Component, h, type ComponentChildren, type FunctionComponent } from "preact";

export function memo<P extends object>(Inner: FunctionComponent<P>): FunctionComponent<P> {
  class Memo extends Component<P> {
    shouldComponentUpdate(next: P) {
      const now = this.props as any, to = next as any;
      for (const k in to) if (to[k] !== now[k]) return true;
      for (const k in now) if (!(k in to)) return true;
      return false;
    }
    render(): ComponentChildren { return h(Inner as any, this.props as any); }
  }
  (Memo as any).displayName = "memo(" + (Inner.displayName || Inner.name) + ")";
  return Memo as unknown as FunctionComponent<P>;
}

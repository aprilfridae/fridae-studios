import { Component } from "preact";
/* a component drawn again only when `same` says its props changed */
export function keep<P>(View: (p: P) => any, same: (a: P, b: P) => boolean) {
  return class extends Component<P> {
    shouldComponentUpdate(n: P) { return !same(this.props, n); }
    render() { return <View {...(this.props as any)} />; }
  };
}

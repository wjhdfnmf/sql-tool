// Shows an element somewhere else on the page than where its component is
// drawn: a result's tools at the end of the results tab row, a connection's
// buttons in the row of connection tabs. The element stays its component's
// to update and to remove.

export function placeIn(node: HTMLElement, host: HTMLElement | undefined) {
  host?.appendChild(node);
  return {
    update(next: HTMLElement | undefined) {
      if (next && node.parentElement !== next) next.appendChild(node);
    },
    destroy() {
      node.remove();
    },
  };
}

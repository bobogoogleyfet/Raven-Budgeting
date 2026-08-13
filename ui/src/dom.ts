/** Small DOM helpers. No framework: the app is four screens and a form. */

type Attrs = Record<string, string | boolean | number>;

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Attrs = {},
  children: (Node | string)[] = [],
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === false) continue;
    if (key === "class") node.className = String(value);
    else if (value === true) node.setAttribute(key, "");
    else node.setAttribute(key, String(value));
  }
  node.append(...children);
  return node;
}

export function clear(node: Element): void {
  node.replaceChildren();
}

/** `document.querySelector`, but a missing element is a bug, not a `null`. */
export function need<E extends Element = HTMLElement>(selector: string): E {
  const node = document.querySelector<E>(selector);
  if (node === null) {
    throw new Error(`missing required element: ${selector}`);
  }
  return node;
}

/**
 * Trailing-edge debounce.
 *
 * The preview runs on every keystroke, and each one is an IPC round trip plus
 * a SQLite read. Without this, typing "154.32" fires six of them and the
 * answers can land out of order, so the panel briefly shows the numbers for
 * "154.3" *after* the ones for "154.32".
 */
export function debounce<A extends unknown[]>(
  ms: number,
  fn: (...args: A) => void,
): (...args: A) => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return (...args: A) => {
    if (timer !== undefined) clearTimeout(timer);
    timer = setTimeout(() => fn(...args), ms);
  };
}

/** Today as `YYYY-MM-DD` in the user's own timezone. */
export function today(): string {
  // Deliberately not `toISOString()`, which is UTC: at 8pm in Salt Lake City
  // that returns tomorrow's date, and a receipt would file into the wrong day
  // — and, at a month boundary, the wrong budget month.
  const now = new Date();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${now.getFullYear()}-${month}-${day}`;
}

/** The current `YYYY-MM`, in the user's own timezone. */
export function thisMonth(): string {
  return today().slice(0, 7);
}

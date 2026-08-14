/**
 * A column filter, the way a spreadsheet does it: a funnel in the header that
 * opens a list of tick boxes, everything ticked to begin with.
 *
 * # Why ticking rather than choosing
 *
 * A single-select filter answers "show me Costco". A column of tick boxes
 * answers "show me everything except Amazon", which is the question a list of
 * expenses is usually interrogated with, and it needs no separate "all" state
 * to be understood: nothing hidden is nothing ticked off.
 *
 * Everything ticked therefore means *no filter at all* rather than "an explicit
 * list of all of them" — see {@link narrowedTo}. That distinction is what keeps
 * a new store appearing in the list the moment it is shopped at, instead of
 * being invisible because it was not among the values ticked last week.
 */

import { el } from "./dom.ts";

/**
 * The funnel every spreadsheet puts in a filtered column header, outlined
 * while the column shows everything and solid once it is hiding rows.
 *
 * Drawn rather than typed. The nearest characters — ⌄, ▼, 🔽 — are a caret or
 * an emoji depending on the platform's font, and one of those is a different
 * control's icon. `currentColor` makes it follow the button's own colour, so
 * the active state and the dark theme both come for free.
 */
const FUNNEL = (filled: boolean): string =>
  `<svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true" focusable="false">` +
  `<path d="M2 3h12l-4.6 5.4v4.2l-2.8 1.4V8.4z" ` +
  (filled
    ? `fill="currentColor"/>`
    : `fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/>`) +
  `</svg>`;

export interface FilterOption {
  /** The id sent back to the core, as a string. */
  value: string;
  label: string;
}

export interface FilterMenu {
  /** The header cell's contents: the label, the funnel, and the popup. */
  readonly root: HTMLElement;
  /** Chosen values, or `null` for "everything" — see {@link narrowedTo}. */
  selected(): string[] | null;
  setOptions(options: FilterOption[]): void;
}

/**
 * What to send to the core: `null` when nothing is hidden.
 *
 * A filter listing every value and no filter at all return the same rows
 * *today*, and different rows tomorrow: an expense at a shop that did not exist
 * when the menu was built would fail the explicit list. Untouched has to mean
 * untouched.
 */
export function narrowedTo(all: readonly string[], hidden: ReadonlySet<string>): string[] | null {
  if (hidden.size === 0) return null;
  return all.filter((value) => !hidden.has(value));
}

export function filterMenu(config: { label: string; onChange: () => void }): FilterMenu {
  const root = el("div", { class: "filter-head" });
  const button = el("button", {
    type: "button",
    class: "filter-toggle",
    title: `Filter by ${config.label.toLowerCase()}`,
    "aria-label": `Filter by ${config.label.toLowerCase()}`,
  });
  const list = el("div", { class: "filter-list", hidden: true });
  const popup = el("div", { class: "filter-popup", hidden: true });

  let options: FilterOption[] = [];
  const hidden = new Set<string>();

  /** The funnel fills in when the column is filtered, so it is visible at a
   * glance which columns are hiding rows. */
  function paint(): void {
    // Static markup, no user data: the labels below are set as text nodes.
    button.innerHTML = FUNNEL(hidden.size > 0);
    button.classList.toggle("active", hidden.size > 0);
  }

  function renderList(): void {
    list.replaceChildren();
    for (const option of options) {
      const box = el("input", { type: "checkbox", value: option.value });
      box.checked = !hidden.has(option.value);
      box.addEventListener("change", () => {
        if (box.checked) hidden.delete(option.value);
        else hidden.add(option.value);
        paint();
        config.onChange();
      });
      list.append(el("label", { class: "filter-row" }, [box, option.label]));
    }
  }

  const all = el("button", { type: "button", class: "link" }, ["Show all"]);
  all.addEventListener("click", () => {
    hidden.clear();
    renderList();
    paint();
    config.onChange();
  });

  const none = el("button", { type: "button", class: "link" }, ["Hide all"]);
  none.addEventListener("click", () => {
    for (const option of options) hidden.add(option.value);
    renderList();
    paint();
    config.onChange();
  });

  popup.append(el("div", { class: "filter-actions" }, [all, none]), list);
  list.hidden = false;

  function close(): void {
    popup.hidden = true;
    document.removeEventListener("mousedown", onOutside, true);
  }

  // Capture, and on mousedown rather than click: a click that lands on another
  // header's funnel must close this popup *and* open that one, which needs the
  // close to happen before the other button's own handler runs.
  function onOutside(event: Event): void {
    if (event.target instanceof Node && root.contains(event.target)) return;
    close();
  }

  button.addEventListener("click", () => {
    if (popup.hidden) {
      popup.hidden = false;
      document.addEventListener("mousedown", onOutside, true);
    } else {
      close();
    }
  });

  root.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && !popup.hidden) close();
  });

  root.append(el("span", { class: "filter-label" }, [config.label]), button, popup);
  paint();

  return {
    root,
    selected: () => narrowedTo(options.map((o) => o.value), hidden),
    setOptions: (next: FilterOption[]) => {
      options = [...next];
      // A value that no longer exists stops hiding anything, so a store that
      // was removed cannot keep filtering the list from beyond the grave.
      for (const value of [...hidden]) {
        if (!options.some((o) => o.value === value)) hidden.delete(value);
      }
      renderList();
      paint();
    },
  };
}

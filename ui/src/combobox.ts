/**
 * A text input with a filtered dropdown — used for the store field.
 *
 * Typing filters; the list is still browsable when the box is empty. A name
 * that matches nothing is kept as typed rather than rejected, because the
 * store you are standing in is more authoritative than the list.
 */

import { caret } from "./icons.ts";

/**
 * Rank `options` against what has been typed.
 *
 * Prefix matches come before interior ones, so typing "ma" offers Macey's
 * before Walmart — the store whose *name* starts that way is nearly always the
 * one meant, and burying it under an alphabetical accident costs a scroll on
 * every entry. Ties keep the input order, which is most-recently-shopped
 * first.
 */
export function filterOptions(options: string[], query: string): string[] {
  const needle = query.trim().toLowerCase();
  if (needle === "") return [...options];

  const prefix: string[] = [];
  const interior: string[] = [];
  for (const option of options) {
    const haystack = option.toLowerCase();
    if (haystack.startsWith(needle)) prefix.push(option);
    else if (haystack.includes(needle)) interior.push(option);
  }
  return [...prefix, ...interior];
}

/** Whether `query` already names an existing option, ignoring case and space. */
export function isKnown(options: string[], query: string): boolean {
  const needle = query.trim().toLowerCase();
  return needle !== "" && options.some((o) => o.trim().toLowerCase() === needle);
}

export interface Combobox {
  readonly root: HTMLElement;
  readonly input: HTMLInputElement;
  value(): string;
  setOptions(options: string[]): void;
  /** Add a name the user invented, so it is offered for the rest of the session. */
  addOption(name: string): void;
  options(): string[];
}

export function combobox(config: {
  id?: string;
  placeholder?: string;
  /** Fired when the value settles: a pick, or Enter, or leaving the field. */
  onCommit?: (value: string) => void;
}): Combobox {
  const root = document.createElement("div");
  root.className = "combo";

  const input = document.createElement("input");
  input.type = "text";
  input.autocomplete = "off";
  input.className = "combo-input";
  if (config.id !== undefined) input.id = config.id;
  if (config.placeholder !== undefined) input.placeholder = config.placeholder;

  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "combo-toggle";
  // The same triangle the Date column sorts with: a dropdown arrow and a
  // sort arrow are the same idea, and two glyphs would be two shapes.
  toggle.innerHTML = caret(true);
  toggle.tabIndex = -1;
  toggle.setAttribute("aria-label", "show all stores");

  const list = document.createElement("div");
  list.className = "combo-list";
  list.hidden = true;

  root.append(input, toggle, list);

  let all: string[] = [];
  let shown: string[] = [];
  // Which row the arrow keys are on. -1 means "none", so Enter takes what was
  // typed rather than whatever happened to be at the top.
  let active = -1;

  function commit(value: string): void {
    input.value = value;
    close();
    config.onCommit?.(value);
  }

  function render(): void {
    shown = filterOptions(all, input.value);
    list.replaceChildren();

    if (shown.length === 0) {
      const empty = document.createElement("div");
      empty.className = "combo-empty";
      empty.textContent =
        input.value.trim() === ""
          ? "No stores yet."
          : `Press Enter to add “${input.value.trim()}”`;
      list.append(empty);
      return;
    }

    shown.forEach((name, index) => {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "combo-option";
      if (index === active) row.classList.add("active");
      row.textContent = name;
      // mousedown, not click: the input's blur would close the list before a
      // click ever landed.
      row.addEventListener("mousedown", (event) => {
        event.preventDefault();
        commit(name);
      });
      list.append(row);
    });
  }

  function open(): void {
    active = -1;
    render();
    list.hidden = false;
  }

  function close(): void {
    list.hidden = true;
    active = -1;
  }

  function move(delta: 1 | -1): void {
    if (list.hidden) open();
    if (shown.length === 0) return;
    active = active + delta;
    if (active < -1) active = shown.length - 1;
    if (active >= shown.length) active = -1;
    render();
  }

  input.addEventListener("focus", () => open());
  input.addEventListener("input", () => {
    active = -1;
    render();
    list.hidden = false;
  });

  toggle.addEventListener("mousedown", (event) => {
    event.preventDefault();
    input.focus();
    if (list.hidden) open();
    else close();
  });

  input.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      move(1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      move(-1);
    } else if (event.key === "Escape") {
      close();
    } else if (event.key === "Enter") {
      event.preventDefault();
      const picked = active >= 0 ? shown[active] : undefined;
      const value = picked ?? input.value.trim();
      if (value === "") {
        close();
        return;
      }
      // A name nobody has used before joins the list immediately, so it is
      // offered for the rest of the session. It only reaches the database on
      // save — a store row for a receipt the user abandoned would be debris in
      // the picker forever.
      if (!isKnown(all, value)) all = [value, ...all];
      commit(value);
    }
  });

  root.addEventListener("focusout", (event) => {
    const next = event.relatedTarget;
    if (!(next instanceof Node) || !root.contains(next)) {
      close();
      config.onCommit?.(input.value.trim());
    }
  });

  return {
    root,
    input,
    value: () => input.value.trim(),
    setOptions: (options: string[]) => {
      all = [...options];
    },
    addOption: (name: string) => {
      if (!isKnown(all, name) && name.trim() !== "") all = [name.trim(), ...all];
    },
    options: () => [...all],
  };
}

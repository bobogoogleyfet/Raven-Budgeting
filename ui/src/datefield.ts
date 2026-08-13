/**
 * A segmented `MM / DD / YYYY` field with a calendar popup.
 *
 * # Why not `<input type="date">`
 *
 * The native control's segment behaviour belongs to the webview engine, and in
 * a WebKit webview it is neither configurable nor the same as the one you get
 * in Chrome. Typing a date is the very first thing you do for every receipt,
 * so it is worth owning outright — including the picker, which went away with
 * the native input.
 *
 * # The rules this implements
 *
 * * **A click anywhere on the field starts at the month** and opens the
 *   calendar. Not "wherever you clicked": the field is small, the segments are
 *   two characters wide, and a click that lands on the day when you meant the
 *   month costs more than always starting from a known place.
 * * **Typing dismisses the calendar.** Someone who has started typing digits
 *   has already decided not to use the picker, and leaving it up covers the
 *   form underneath.
 * * Focusing a segment selects it whole, so typing overwrites. The first digit
 *   replaces the selection and drops it, leaving the caret after that digit —
 *   that is what makes Backspace remove one character rather than wiping all
 *   four digits of a year the user only meant to correct.
 * * A segment that fills up (2, 2, 4 digits) hands focus to the next and
 *   selects it. Backspace in an empty segment steps back; arrows step at the
 *   edges; Enter leaves the field.
 *
 * Three real `<input>` elements rather than one with hand-managed carets: the
 * selection behaviour above is exactly what a short native input already does,
 * so this steers focus and lets the platform handle the text.
 */

import { WEEKDAYS, monthGrid, monthLabel, shiftMonth } from "./calendar.ts";

/** The three parts, in the order they are typed. */
const LENGTHS = { month: 2, day: 2, year: 4 } as const;

type SegmentName = keyof typeof LENGTHS;

const ORDER: SegmentName[] = ["month", "day", "year"];

/**
 * Assemble typed segments into `YYYY-MM-DD`, or `null` if they are not a real
 * date.
 *
 * Rejects the impossible rather than clamping it. Silently turning 2026-02-30
 * into 2026-02-28 would file a receipt on a day the user did not choose, and
 * on a month boundary that is the wrong budget month.
 */
export function toIso(month: string, day: string, year: string): string | null {
  if (year.length !== 4 || month.length === 0 || day.length === 0) return null;

  const y = Number(year);
  const m = Number(month);
  const d = Number(day);
  if (!Number.isInteger(y) || !Number.isInteger(m) || !Number.isInteger(d)) return null;
  if (y < 1 || m < 1 || m > 12 || d < 1 || d > 31) return null;

  // Round-tripping through Date is what catches February 30th and the leap
  // year rule without a table: the constructor rolls an out-of-range day into
  // the next month, so a mismatch means the date does not exist.
  const probe = new Date(y, m - 1, d);
  if (probe.getFullYear() !== y || probe.getMonth() !== m - 1 || probe.getDate() !== d) {
    return null;
  }

  const pad = (n: number, width: number): string => String(n).padStart(width, "0");
  return `${pad(y, 4)}-${pad(m, 2)}-${pad(d, 2)}`;
}

/** Split `YYYY-MM-DD` into display segments. Junk yields empty segments. */
export function fromIso(iso: string): { month: string; day: string; year: string } {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
  if (match === null) return { month: "", day: "", year: "" };
  const [, year = "", month = "", day = ""] = match;
  return { month, day, year };
}

export interface DateField {
  readonly root: HTMLElement;
  /** `YYYY-MM-DD`, or `""` while the date is incomplete or impossible. */
  value(): string;
  setValue(iso: string): void;
  /** Focus the month segment with it selected — the "start here" state. */
  focusStart(): void;
}

export function dateField(options: {
  id?: string;
  onChange?: () => void;
  /** Enter moves on: the field is done, the form is not. */
  onEnter?: () => void;
}): DateField {
  const root = document.createElement("div");
  root.className = "datefield-wrap";

  const field = document.createElement("div");
  field.className = "datefield";
  root.append(field);

  const popup = document.createElement("div");
  popup.className = "calendar";
  popup.hidden = true;
  root.append(popup);

  const inputs = {} as Record<SegmentName, HTMLInputElement>;

  ORDER.forEach((name, index) => {
    if (index > 0) {
      const sep = document.createElement("span");
      sep.className = "datefield-sep";
      sep.textContent = "/";
      field.append(sep);
    }

    const input = document.createElement("input");
    input.type = "text";
    input.inputMode = "numeric";
    input.autocomplete = "off";
    input.className = `datefield-seg seg-${name}`;
    input.maxLength = LENGTHS[name];
    input.placeholder = name === "year" ? "YYYY" : name === "month" ? "MM" : "DD";
    input.setAttribute("aria-label", name);
    if (index === 0 && options.id !== undefined) input.id = options.id;
    inputs[name] = input;
    field.append(input);
  });

  function currentIso(): string {
    return toIso(inputs.month.value, inputs.day.value, inputs.year.value) ?? "";
  }

  function focusSegment(name: SegmentName): void {
    const input = inputs[name];
    input.focus();
    // Selecting on arrival is what makes typing overwrite; the browser drops
    // the selection itself as soon as the first character replaces it.
    input.select();
  }

  function step(from: SegmentName, delta: 1 | -1): void {
    const next = ORDER[ORDER.indexOf(from) + delta];
    if (next !== undefined) focusSegment(next);
  }

  // --- calendar -------------------------------------------------------------

  // What the popup is showing, which is not the same as what is selected: you
  // can page to April while the field still holds a March date.
  let shownYear = 0;
  let shownMonth = 0;

  function renderCalendar(): void {
    popup.replaceChildren();
    const selected = currentIso();

    const header = document.createElement("div");
    header.className = "calendar-head";

    const back = document.createElement("button");
    back.type = "button";
    back.className = "calendar-nav";
    back.textContent = "‹";
    back.setAttribute("aria-label", "previous month");
    back.addEventListener("mousedown", (e) => {
      // Without this the field loses focus to the button and the popup's
      // focusout handler closes it mid-click.
      e.preventDefault();
      [shownYear, shownMonth] = shiftMonth(shownYear, shownMonth, -1);
      renderCalendar();
    });

    const forward = document.createElement("button");
    forward.type = "button";
    forward.className = "calendar-nav";
    forward.textContent = "›";
    forward.setAttribute("aria-label", "next month");
    forward.addEventListener("mousedown", (e) => {
      e.preventDefault();
      [shownYear, shownMonth] = shiftMonth(shownYear, shownMonth, 1);
      renderCalendar();
    });

    const label = document.createElement("span");
    label.className = "calendar-label";
    label.textContent = monthLabel(shownYear, shownMonth);

    header.append(back, label, forward);
    popup.append(header);

    const grid = document.createElement("div");
    grid.className = "calendar-grid";
    for (const name of WEEKDAYS) {
      const cell = document.createElement("span");
      cell.className = "calendar-weekday";
      cell.textContent = name;
      grid.append(cell);
    }

    const todayIso = (() => {
      const now = new Date();
      const pad = (n: number): string => String(n).padStart(2, "0");
      return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
    })();

    for (const cell of monthGrid(shownYear, shownMonth)) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "calendar-day";
      if (!cell.inMonth) button.classList.add("outside");
      if (cell.iso === todayIso) button.classList.add("today");
      if (cell.iso === selected) button.classList.add("selected");
      button.textContent = String(cell.day);
      button.addEventListener("mousedown", (e) => {
        e.preventDefault();
        setSegments(cell.iso);
        options.onChange?.();
        closeCalendar();
        // Focus back on the field so the next Tab or Enter behaves as if the
        // date had been typed.
        focusSegment("month");
      });
      grid.append(button);
    }

    popup.append(grid);
  }

  function openCalendar(): void {
    const current = currentIso();
    if (current === "") {
      const now = new Date();
      shownYear = now.getFullYear();
      shownMonth = now.getMonth() + 1;
    } else {
      const parts = fromIso(current);
      shownYear = Number(parts.year);
      shownMonth = Number(parts.month);
    }
    renderCalendar();
    popup.hidden = false;
  }

  function closeCalendar(): void {
    popup.hidden = true;
  }

  function setSegments(iso: string): void {
    const parts = fromIso(iso);
    inputs.month.value = parts.month;
    inputs.day.value = parts.day;
    inputs.year.value = parts.year;
  }

  // --- segment behaviour ----------------------------------------------------

  for (const name of ORDER) {
    const input = inputs[name];

    input.addEventListener("focus", () => input.select());

    input.addEventListener("input", () => {
      // Digits only. Stripping rather than blocking keeps paste working.
      const digits = input.value.replace(/\D/g, "").slice(0, LENGTHS[name]);
      if (digits !== input.value) input.value = digits;
      options.onChange?.();
      if (digits.length === LENGTHS[name]) step(name, 1);
    });

    input.addEventListener("keydown", (event) => {
      // Typing means the picker is not wanted. Checked before anything else so
      // it applies to every digit, wherever the caret is.
      if (/^\d$/.test(event.key)) closeCalendar();

      if (event.key === "Enter") {
        event.preventDefault();
        closeCalendar();
        options.onEnter?.();
        return;
      }
      if (event.key === "Escape") {
        closeCalendar();
        return;
      }

      const atStart = input.selectionStart === 0 && input.selectionEnd === 0;
      const atEnd =
        input.selectionStart === input.value.length && input.selectionEnd === input.value.length;

      if (event.key === "Backspace" && input.value === "") {
        event.preventDefault();
        const previous = ORDER[ORDER.indexOf(name) - 1];
        if (previous !== undefined) {
          const target = inputs[previous];
          target.focus();
          // Caret at the end, not selected: Backspace should now take one
          // character off the previous segment, not erase it.
          const end = target.value.length;
          target.setSelectionRange(end, end);
        }
        return;
      }
      if (event.key === "ArrowLeft" && atStart) {
        event.preventDefault();
        step(name, -1);
      } else if (event.key === "ArrowRight" && atEnd) {
        event.preventDefault();
        step(name, 1);
      }
    });

    // Pad on the way out so "3" becomes "03" rather than an unreadable date.
    input.addEventListener("blur", () => {
      if (input.value !== "" && name !== "year") {
        input.value = input.value.padStart(LENGTHS[name], "0");
        options.onChange?.();
      }
    });
  }

  // A click anywhere on the field — a segment, a separator, the padding —
  // starts at the month and raises the picker. preventDefault stops the
  // browser placing its own caret in whichever segment was under the pointer.
  field.addEventListener("mousedown", (event) => {
    event.preventDefault();
    focusSegment("month");
    openCalendar();
  });

  // Leaving the whole component closes the picker. relatedTarget is where
  // focus went; a click on a calendar day never lands here because those
  // buttons preventDefault on mousedown.
  root.addEventListener("focusout", (event) => {
    const next = event.relatedTarget;
    if (!(next instanceof Node) || !root.contains(next)) closeCalendar();
  });

  return {
    root,
    value: currentIso,
    setValue: setSegments,
    focusStart: () => focusSegment("month"),
  };
}

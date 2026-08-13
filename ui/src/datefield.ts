/**
 * A segmented `MM / DD / YYYY` field.
 *
 * # Why not `<input type="date">`
 *
 * The native control's segment behaviour belongs to the browser engine, and in
 * a WebKit webview it is neither configurable nor the same as the one you get
 * in Chrome. Typing a date is the very first thing you do for every receipt,
 * so it is worth owning outright.
 *
 * # The rules this implements
 *
 * * Focusing a segment selects it whole, so typing overwrites rather than
 *   appending to whatever was there.
 * * The first digit typed replaces the selection and drops it. The caret then
 *   sits after that digit, which is what makes Backspace remove one character:
 *   with the segment still selected, Backspace would wipe all four digits of a
 *   year when the user only meant to correct the last one.
 * * A segment that fills up (2, 2, 4 digits) hands focus to the next one, and
 *   selects it.
 * * Backspace in an empty segment steps back to the previous one; Arrow keys
 *   step at the edges. Enter leaves the field entirely.
 *
 * Three real `<input>` elements rather than one with hand-managed carets: the
 * selection and caret behaviour above is exactly what a short native input
 * already does, so this steers focus and lets the platform handle the text.
 */

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
  root.className = "datefield";

  const inputs = {} as Record<SegmentName, HTMLInputElement>;

  ORDER.forEach((name, index) => {
    if (index > 0) {
      const sep = document.createElement("span");
      sep.className = "datefield-sep";
      sep.textContent = "/";
      root.append(sep);
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
    root.append(input);
  });

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

  for (const name of ORDER) {
    const input = inputs[name];

    // Clicking a segment starts a fresh entry for it rather than dropping a
    // caret mid-number.
    input.addEventListener("focus", () => input.select());
    input.addEventListener("mouseup", (event) => {
      // Chrome/WebKit clear the selection on mouseup after focus; re-selecting
      // here is what keeps click-then-type an overwrite.
      event.preventDefault();
      input.select();
    });

    input.addEventListener("input", () => {
      // Digits only. Stripping rather than blocking keeps paste working.
      const digits = input.value.replace(/\D/g, "").slice(0, LENGTHS[name]);
      if (digits !== input.value) input.value = digits;
      options.onChange?.();
      if (digits.length === LENGTHS[name]) step(name, 1);
    });

    input.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        options.onEnter?.();
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

  // A click on the separators or the padding — "into the field" rather than
  // onto a number — starts at the front.
  root.addEventListener("mousedown", (event) => {
    if (event.target === root || (event.target as HTMLElement).classList.contains("datefield-sep")) {
      event.preventDefault();
      focusSegment("month");
    }
  });

  return {
    root,
    value: () => toIso(inputs.month.value, inputs.day.value, inputs.year.value) ?? "",
    setValue: (iso: string) => {
      const parts = fromIso(iso);
      inputs.month.value = parts.month;
      inputs.day.value = parts.day;
      inputs.year.value = parts.year;
    },
    focusStart: () => focusSegment("month"),
  };
}

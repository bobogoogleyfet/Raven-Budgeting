/**
 * The month grid behind the date field's picker.
 *
 * Pure on purpose: laying out a month is where off-by-one errors live — the
 * leading blank count, the leap-February, the month that needs a sixth row —
 * and none of that is worth debugging through a popup. The DOM half in
 * `datefield.ts` only renders what this returns.
 */

/** One cell of the grid. Cells outside the shown month are still real dates. */
export interface DayCell {
  /** `YYYY-MM-DD`. */
  iso: string;
  day: number;
  /** False for the leading/trailing days borrowed from adjacent months. */
  inMonth: boolean;
}

export const WEEKDAYS = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"] as const;

function iso(year: number, month: number, day: number): string {
  const pad = (n: number, w: number): string => String(n).padStart(w, "0");
  return `${pad(year, 4)}-${pad(month, 2)}-${pad(day, 2)}`;
}

/**
 * Six weeks of cells for `month` (1-12) of `year`, Sunday first.
 *
 * Always 42 cells, never a ragged 28-to-42 range. A grid that changes height
 * with the month makes the popup jump as you page through it, and moves the
 * day you were aiming at out from under the pointer.
 */
export function monthGrid(year: number, month: number): DayCell[] {
  const first = new Date(year, month - 1, 1);
  // getDay() is already Sunday-0, which is the order WEEKDAYS is written in.
  const lead = first.getDay();

  const cells: DayCell[] = [];
  for (let i = 0; i < 42; i++) {
    // Letting Date normalise the overflow is what handles both ends at once:
    // day 0 is the last of the previous month, day 32 rolls into the next.
    const d = new Date(year, month - 1, 1 - lead + i);
    cells.push({
      iso: iso(d.getFullYear(), d.getMonth() + 1, d.getDate()),
      day: d.getDate(),
      inMonth: d.getMonth() === month - 1 && d.getFullYear() === year,
    });
  }
  return cells;
}

/** `2026-03` → `"March 2026"`. */
export function monthLabel(year: number, month: number): string {
  const names = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
  ];
  return `${names[month - 1] ?? "?"} ${year}`;
}

/** Step a (year, month) pair by whole months, rolling the year over. */
export function shiftMonth(year: number, month: number, delta: number): [number, number] {
  const zero = year * 12 + (month - 1) + delta;
  return [Math.floor(zero / 12), (((zero % 12) + 12) % 12) + 1];
}

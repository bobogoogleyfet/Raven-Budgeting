import assert from "node:assert/strict";
import { test } from "node:test";

import { type DayCell, monthGrid, monthLabel, shiftMonth } from "./calendar.ts";

function inMonth(cells: DayCell[]): DayCell[] {
  return cells.filter((c) => c.inMonth);
}

test("the grid is always six weeks", () => {
  // A grid that changes height makes the popup jump as you page through it,
  // moving the day you were aiming at out from under the pointer.
  for (const [y, m] of [
    [2026, 2],
    [2026, 3],
    [2026, 8],
    [2028, 2],
  ] as const) {
    assert.equal(monthGrid(y, m).length, 42, `${y}-${m}`);
  }
});

test("a month starting on Sunday has no leading blanks", () => {
  // 2026-03-01 is a Sunday.
  const cells = monthGrid(2026, 3);
  assert.equal(cells[0]?.iso, "2026-03-01");
  assert.equal(cells[0]?.inMonth, true);
});

test("a month starting mid-week borrows the right number of leading days", () => {
  // 2026-08-01 is a Saturday, so six days of July lead it.
  const cells = monthGrid(2026, 8);
  assert.equal(cells[0]?.iso, "2026-07-26");
  assert.equal(cells[0]?.inMonth, false);
  assert.equal(cells[6]?.iso, "2026-08-01");
  assert.equal(cells[6]?.inMonth, true);
});

test("the in-month days are exactly the month's days, in order", () => {
  const march = inMonth(monthGrid(2026, 3));
  assert.equal(march.length, 31);
  assert.equal(march[0]?.iso, "2026-03-01");
  assert.equal(march[30]?.iso, "2026-03-31");

  const november = inMonth(monthGrid(2026, 11));
  assert.equal(november.length, 30);
  assert.equal(november[29]?.iso, "2026-11-30");
});

test("February knows about leap years", () => {
  assert.equal(inMonth(monthGrid(2026, 2)).length, 28);
  assert.equal(inMonth(monthGrid(2028, 2)).length, 29);
  assert.equal(inMonth(monthGrid(2000, 2)).length, 29); // divisible by 400
  assert.equal(inMonth(monthGrid(1900, 2)).length, 28); // divisible by 100, not 400
});

test("2028-02-01 is a Tuesday, so two January days lead it", () => {
  const cells = monthGrid(2028, 2);
  assert.equal(cells[0]?.iso, "2028-01-30");
  assert.equal(cells[1]?.iso, "2028-01-31");
  assert.equal(cells[2]?.iso, "2028-02-01");
  assert.equal(cells[2]?.inMonth, true);
});

test("every cell is one day after the last, across both month seams", () => {
  const cells = monthGrid(2026, 8);
  for (let i = 1; i < cells.length; i++) {
    const previous = new Date(`${cells[i - 1]?.iso}T00:00:00`);
    const current = new Date(`${cells[i]?.iso}T00:00:00`);
    const days = (current.getTime() - previous.getTime()) / 86_400_000;
    assert.equal(days, 1, `gap at cell ${i}`);
  }
});

test("stepping months rolls the year in both directions", () => {
  assert.deepEqual(shiftMonth(2026, 12, 1), [2027, 1]);
  assert.deepEqual(shiftMonth(2026, 1, -1), [2025, 12]);
  assert.deepEqual(shiftMonth(2026, 3, 0), [2026, 3]);
  assert.deepEqual(shiftMonth(2026, 1, -13), [2024, 12]);
  assert.deepEqual(shiftMonth(2026, 6, 18), [2027, 12]);
});

test("the label names the month", () => {
  assert.equal(monthLabel(2026, 3), "March 2026");
  assert.equal(monthLabel(2026, 12), "December 2026");
});

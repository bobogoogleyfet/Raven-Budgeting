import assert from "node:assert/strict";
import { test } from "node:test";

import { fromIso, toIso } from "./datefield.ts";

test("typed segments assemble into an ISO date", () => {
  assert.equal(toIso("03", "14", "2026"), "2026-03-14");
  assert.equal(toIso("12", "31", "2026"), "2026-12-31");
});

test("single-digit months and days are padded", () => {
  // The user types "3" and tabs away; the field pads, and the ISO date must
  // still be well formed for the core's YYYY-MM-DD check.
  assert.equal(toIso("3", "4", "2026"), "2026-03-04");
});

test("an incomplete date is null rather than a guess", () => {
  assert.equal(toIso("03", "14", "202"), null); // year still being typed
  assert.equal(toIso("", "14", "2026"), null);
  assert.equal(toIso("03", "", "2026"), null);
});

test("an impossible date is rejected, not clamped", () => {
  // Clamping Feb 30 to Feb 28 would file the receipt on a day the user did
  // not pick — and at a month boundary, into the wrong budget month.
  assert.equal(toIso("02", "30", "2026"), null);
  assert.equal(toIso("04", "31", "2026"), null);
  assert.equal(toIso("13", "01", "2026"), null);
  assert.equal(toIso("00", "10", "2026"), null);
  assert.equal(toIso("01", "00", "2026"), null);
  assert.equal(toIso("01", "32", "2026"), null);
});

test("leap years are handled by the calendar, not a table", () => {
  assert.equal(toIso("02", "29", "2028"), "2028-02-29"); // divisible by 4
  assert.equal(toIso("02", "29", "2026"), null); // not a leap year
  assert.equal(toIso("02", "29", "2000"), "2000-02-29"); // divisible by 400
  assert.equal(toIso("02", "29", "1900"), null); // divisible by 100, not 400
});

test("an ISO date splits back into segments", () => {
  assert.deepEqual(fromIso("2026-03-14"), { month: "03", day: "14", year: "2026" });
});

test("junk splits into empty segments rather than throwing", () => {
  for (const input of ["", "nope", "2026-3-4", "03/14/2026"]) {
    assert.deepEqual(fromIso(input), { month: "", day: "", year: "" }, `on ${JSON.stringify(input)}`);
  }
});

test("a date round-trips through both directions", () => {
  for (const iso of ["2026-01-01", "2026-12-31", "2028-02-29", "1999-06-15"]) {
    const p = fromIso(iso);
    assert.equal(toIso(p.month, p.day, p.year), iso);
  }
});

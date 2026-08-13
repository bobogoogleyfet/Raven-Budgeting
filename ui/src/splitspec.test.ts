import assert from "node:assert/strict";
import { test } from "node:test";

import { parseSplitAmount } from "./splitspec.ts";

test("an empty last row means the rest, which is what a one-category receipt needs", () => {
  assert.deepEqual(parseSplitAmount("", true), { kind: "remainder" });
  assert.deepEqual(parseSplitAmount("   ", true), { kind: "remainder" });
});

test("an empty earlier row is incomplete, not a zero split", () => {
  // Returning a $0.00 split would claim the user meant to file nothing under
  // that category; dropping the row leaves the money unassigned, which the
  // core reports as a blocker and the form shows.
  assert.equal(parseSplitAmount("", false), null);
});

test("a plain amount is fixed cents", () => {
  assert.deepEqual(parseSplitAmount("45.00", true), { kind: "fixed", cents: 4500 });
  assert.deepEqual(parseSplitAmount("$1,234.56", false), { kind: "fixed", cents: 123_456 });
  assert.deepEqual(parseSplitAmount(" 30 ", false), { kind: "fixed", cents: 3000 });
});

test("a percentage becomes basis points", () => {
  assert.deepEqual(parseSplitAmount("25%", false), { kind: "percent", bp: 2500 });
  // 33.33% is 3333 bp exactly — the case where a float would drift.
  assert.deepEqual(parseSplitAmount("33.33%", false), { kind: "percent", bp: 3333 });
  assert.deepEqual(parseSplitAmount("100%", false), { kind: "percent", bp: 10_000 });
  assert.deepEqual(parseSplitAmount("0%", false), { kind: "percent", bp: 0 });
});

test("a malformed percentage is refused rather than read as zero", () => {
  // Number("") and Number(" ") are both 0, so a bare "%" would silently
  // become a 0% split without this.
  for (const input of ["%", " %", "abc%", "1.2.3%", "-5%", "101%"]) {
    assert.equal(parseSplitAmount(input, false), null, `expected ${input} to be refused`);
  }
});

test("junk in an amount box is refused", () => {
  for (const input of ["abc", "1.2.3", "$", "1a"]) {
    assert.equal(parseSplitAmount(input, false), null, `expected ${input} to be refused`);
  }
});

test("a third decimal is refused here too, via parseCents", () => {
  assert.equal(parseSplitAmount("1.005", false), null);
});

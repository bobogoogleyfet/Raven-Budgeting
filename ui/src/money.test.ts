/**
 * Tests for the display/parse boundary.
 *
 * These matter more than their size suggests: this is the only JavaScript in
 * the app allowed anywhere near a monetary value, so a bug here is a bug the
 * Rust test suite structurally cannot catch.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  ZERO,
  formatCents,
  formatMoney,
  formatRatePpm,
  fromCore,
  parseCents,
  rateSummary,
} from "./money.ts";

test("formatting pads the cents rather than truncating them", () => {
  assert.equal(formatCents(fromCore(5)), "0.05");
  assert.equal(formatCents(fromCore(50)), "0.50");
  assert.equal(formatCents(fromCore(100)), "1.00");
  assert.equal(formatCents(ZERO), "0.00");
});

test("formatting groups thousands and keeps the sign outside the symbol", () => {
  assert.equal(formatCents(fromCore(123_456)), "1,234.56");
  assert.equal(formatMoney(fromCore(123_456)), "$1,234.56");
  // "-$5.00", not "$-5.00": the sign belongs to the amount, not the currency.
  assert.equal(formatMoney(fromCore(-500)), "-$5.00");
  // A small negative must not format as "-0.-5".
  assert.equal(formatCents(fromCore(-5)), "-0.05");
});

test("parsing accepts what people actually type", () => {
  assert.equal(parseCents("12.34"), 1234);
  assert.equal(parseCents("$12.34"), 1234);
  assert.equal(parseCents(" 1,234.56 "), 123_456);
  assert.equal(parseCents("12"), 1200);
  assert.equal(parseCents("12."), 1200);
  assert.equal(parseCents(".5"), 50);
  assert.equal(parseCents("0.07"), 7);
  assert.equal(parseCents("-5.00"), -500);
});

test("a third decimal place is refused rather than silently rounded", () => {
  // Rounding 1.005 to 1.00 would take a cent off the user without saying so,
  // and there is no half cent on a receipt.
  assert.equal(parseCents("1.005"), null);
  assert.equal(parseCents("0.001"), null);
});

test("an incomplete or junk amount parses as null, not as zero", () => {
  // These are keystroke states, not errors — but they must never be mistaken
  // for a real $0.00, which is a meaningful amount on a tax field.
  for (const input of ["", "   ", "-", "abc", "1.2.3", "$", "1a"]) {
    assert.equal(parseCents(input), null, `expected ${JSON.stringify(input)} to parse as null`);
  }
});

test("parsing is exact where floating point is not", () => {
  // "0.07" * 100 is 7.000000000000001 in IEEE 754. String assembly avoids it.
  assert.equal(parseCents("0.07"), 7);
  assert.equal(parseCents("0.29"), 29);
  assert.equal(parseCents("1.10"), 110);
  // Round-tripping every cent of the first two hundred dollars.
  for (let cents = 0; cents <= 20_000; cents++) {
    const text = formatCents(fromCore(cents));
    assert.equal(parseCents(text), cents, `round trip failed at ${cents}`);
  }
});

test("the core sending a non-integer is an error, not a rendered guess", () => {
  assert.throws(() => fromCore(12.34), TypeError);
  assert.throws(() => fromCore(Number.NaN), TypeError);
});

test("rates render exactly, including the awkward ones", () => {
  assert.equal(formatRatePpm(83_750), "8.375%");
  // 9.4375% is the case that percent-with-2-decimals rounds away.
  assert.equal(formatRatePpm(94_375), "9.4375%");
  assert.equal(formatRatePpm(70_000), "7%");
  assert.equal(formatRatePpm(0), "0%");
});

test("a reduced food rate is named alongside the general one", () => {
  assert.equal(rateSummary(74_500, 30_000), "7.45% · groceries 3%");
});

test("no reduced rate says nothing about groceries", () => {
  // Most states tax food in full. Printing "groceries 7.25%" would imply a
  // rule that does not exist.
  assert.equal(rateSummary(74_500, null), "7.45%");
});

test("a food rate equal to the general one is not a reduction", () => {
  assert.equal(rateSummary(74_500, 74_500), "7.45%");
});

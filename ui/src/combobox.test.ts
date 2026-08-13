import assert from "node:assert/strict";
import { test } from "node:test";

import { filterOptions, isKnown } from "./combobox.ts";

const STORES = [
  "Costco",
  "Macey's",
  "Walmart",
  "Smith's",
  "Harmons",
  "Maverik",
  "Amazon",
  "Target",
];

test("an empty query offers everything, in the order given", () => {
  // The order is most-recently-shopped first, so it must survive untouched.
  assert.deepEqual(filterOptions(STORES, ""), STORES);
  assert.deepEqual(filterOptions(STORES, "   "), STORES);
});

test("typing filters to matches", () => {
  assert.deepEqual(filterOptions(STORES, "cost"), ["Costco"]);
  assert.deepEqual(filterOptions(STORES, "harm"), ["Harmons"]);
});

test("prefix matches come before interior ones", () => {
  // "ma" starts Macey's and Maverik, and sits inside Wal-ma-rt and A-ma-zon.
  // The store whose name *starts* that way is nearly always the one meant, so
  // both prefix hits come first; within each group the input order (most
  // recently shopped) is kept.
  assert.deepEqual(filterOptions(STORES, "ma"), ["Macey's", "Maverik", "Walmart", "Amazon"]);
});

test("filtering ignores case and surrounding space", () => {
  assert.deepEqual(filterOptions(STORES, "COSTCO"), ["Costco"]);
  assert.deepEqual(filterOptions(STORES, "  amazon "), ["Amazon"]);
});

test("apostrophes and punctuation in a name are matchable", () => {
  assert.deepEqual(filterOptions(STORES, "macey's"), ["Macey's"]);
  assert.deepEqual(filterOptions(STORES, "ey's"), ["Macey's"]);
});

test("a query matching nothing yields nothing, which is what offers to add it", () => {
  assert.deepEqual(filterOptions(STORES, "Trader Joe's"), []);
});

test("a known name is recognised regardless of case or padding", () => {
  assert.equal(isKnown(STORES, "costco"), true);
  assert.equal(isKnown(STORES, "  Walmart  "), true);
  assert.equal(isKnown(STORES, "Macey's"), true);
});

test("an unknown or empty name is not known", () => {
  assert.equal(isKnown(STORES, "Trader Joe's"), false);
  assert.equal(isKnown(STORES, ""), false);
  assert.equal(isKnown(STORES, "   "), false);
  // A prefix is not a match: "Cost" must still offer to be added, because
  // "Cost Plus" is a different shop from Costco.
  assert.equal(isKnown(STORES, "Cost"), false);
});

test("filtering never mutates the source list", () => {
  const original = [...STORES];
  filterOptions(STORES, "ma");
  assert.deepEqual(STORES, original);
});

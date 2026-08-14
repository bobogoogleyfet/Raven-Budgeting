import assert from "node:assert/strict";
import { test } from "node:test";

import { narrowedTo } from "./filtermenu.ts";

const STORES = ["1", "2", "3"];

test("nothing hidden is no filter at all", () => {
  // Not "all three ids". A filter listing every value and no filter return the
  // same rows today and different rows tomorrow: an expense at a shop that did
  // not exist when the menu was built would fail the explicit list.
  assert.equal(narrowedTo(STORES, new Set()), null);
});

test("hiding one leaves the rest", () => {
  assert.deepEqual(narrowedTo(STORES, new Set(["2"])), ["1", "3"]);
});

test("the order of the source list is kept", () => {
  // The menu is built alphabetically, and the query should not reshuffle it.
  assert.deepEqual(narrowedTo(STORES, new Set(["1"])), ["2", "3"]);
});

test("hiding everything asks for nothing, which is not the same as everything", () => {
  // An empty list means "no rows match", and the caller must send it as such.
  // Collapsing it back to null would make Hide All show the whole table.
  assert.deepEqual(narrowedTo(STORES, new Set(STORES)), []);
});

test("a hidden value that is no longer offered does not narrow anything", () => {
  assert.deepEqual(narrowedTo(STORES, new Set(["9"])), STORES);
});

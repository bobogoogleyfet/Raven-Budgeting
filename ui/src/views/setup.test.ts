import assert from "node:assert/strict";
import { test } from "node:test";

import { parseRatePpm } from "./setup.ts";

test("a percentage becomes parts per million", () => {
  assert.equal(parseRatePpm("7.75"), 77_500);
  assert.equal(parseRatePpm("0"), 0);
  assert.equal(parseRatePpm("10"), 100_000);
});

test("a trailing percent sign and padding are accepted", () => {
  assert.equal(parseRatePpm(" 7.75% "), 77_500);
  assert.equal(parseRatePpm("7.75%"), 77_500);
});

test("four decimal places survive, which is the point of ppm", () => {
  // 9.4375% is the rate that percent-with-2-decimals rounds away.
  assert.equal(parseRatePpm("9.4375"), 94_375);
});

test("a rate outside 0-100% is refused", () => {
  assert.equal(parseRatePpm("-1"), null);
  assert.equal(parseRatePpm("101"), null);
});

test("junk is refused rather than read as zero", () => {
  // Number("") and Number(" ") are both 0, so a blank field would otherwise
  // silently configure a 0% rate — exactly the state this prompt exists to
  // prevent.
  for (const input of ["", "   ", "%", "abc", "7.7.5", "seven"]) {
    assert.equal(parseRatePpm(input), null, `expected ${JSON.stringify(input)} to be refused`);
  }
});

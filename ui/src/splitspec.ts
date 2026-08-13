/**
 * Turning what someone typed in a split's amount box into a `SplitAmount`.
 *
 * Extracted from the form so it can be tested: it is the only real logic in
 * the UI, and every other line up there is DOM assembly. Getting it wrong is
 * quiet — a mistyped percentage still saves, it just files the wrong amount
 * under the wrong envelope.
 *
 * Note what it does *not* do: work out what "the rest" is worth. That is the
 * core's job, and asking for it by name (`remainder`) rather than computing
 * `total - others` here is what keeps the split arithmetic in one place.
 */

import type { SplitAmount } from "./api.ts";
import { parseCents } from "./money.ts";

/**
 * Parse one split amount.
 *
 * `isLast` decides what an empty box means: on the final row it is "whatever
 * is left", which is what lets a single-category receipt need no amount typed
 * at all. On any earlier row an empty box is simply incomplete, and returning
 * `null` drops it from the draft rather than inventing a zero — a $0.00 split
 * would claim the user meant to file nothing under that category.
 */
export function parseSplitAmount(text: string, isLast: boolean): SplitAmount | null {
  const trimmed = text.trim();
  if (trimmed === "") {
    return isLast ? { kind: "remainder" } : null;
  }

  if (trimmed.endsWith("%")) {
    const body = trimmed.slice(0, -1).trim();
    // Reject anything that is not plainly a number: `Number("")` is 0 and
    // `Number(" ")` is 0, so a bare "%" would otherwise become 0%.
    if (!/^\d*\.?\d+$/.test(body)) return null;
    const percent = Number(body);
    if (!Number.isFinite(percent) || percent < 0 || percent > 100) return null;
    // Basis points: 25% is 2500, 33.33% is 3333. Integers, so thirds of a
    // receipt stay exact instead of drifting a cent.
    return { kind: "percent", bp: Math.round(percent * 100) };
  }

  const cents = parseCents(trimmed);
  return cents === null ? null : { kind: "fixed", cents };
}

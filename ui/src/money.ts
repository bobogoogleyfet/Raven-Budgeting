/**
 * Money in the frontend: display only.
 *
 * # The rule this file exists to enforce
 *
 * The core owns every monetary calculation. This layer receives integer cents,
 * formats them for a human, and parses what a human typed back into integer
 * cents so it can be sent straight back. It never adds, subtracts, multiplies,
 * apportions, or compares-with-tolerance.
 *
 * That is not fussiness. `0.1 + 0.2 === 0.30000000000000004` in every
 * JavaScript engine, so a frontend that computes `total - subtotal` produces a
 * number that disagrees with the database by a cent at unpredictable moments —
 * and the disagreement surfaces as a receipt that will not save, with no
 * visible cause. The second cost is portability: money logic written here has
 * to be written again for Android and again for iOS, and the three copies drift.
 * `Money::allocate` in the core already guarantees that split parts sum to the
 * whole exactly; re-deriving any of that here throws the guarantee away.
 *
 * # How the type stops it
 *
 * {@link Cents} is a branded integer. Arithmetic on it still type-checks — TS
 * cannot forbid `-` on a number — but the *result* is a plain `number` and has
 * lost the brand, so it cannot be passed back into anything that wants `Cents`.
 * The compiler turns "I did math on money" into an error at the point the
 * result is used, which is the point that matters.
 */

declare const CENTS: unique symbol;

/**
 * An integer number of cents, as the core sends it.
 *
 * Values arrive over IPC as plain JSON integers (`Money` is
 * `serde(transparent)` over `i64`), so no decoding step is needed — only the
 * brand, which exists purely to make misuse a compile error.
 */
export type Cents = number & { readonly [CENTS]: true };

/** The zero amount, for initialising a field. */
export const ZERO = 0 as Cents;

/**
 * Trust an integer from the core as `Cents`.
 *
 * Call this at the IPC boundary and nowhere else. It is deliberately the only
 * way to mint a `Cents`, so every value in the app is traceable to something
 * the core computed or to {@link parseCents}.
 */
export function fromCore(value: number): Cents {
  if (!Number.isSafeInteger(value)) {
    // A non-integer here means the core sent a float, which would mean the
    // i64-cents invariant broke on the Rust side. Failing loudly beats
    // rendering a plausible wrong number.
    throw new TypeError(`expected integer cents from the core, got ${value}`);
  }
  return value as Cents;
}

/** `123456` → `"1,234.56"`. No currency symbol; see {@link formatMoney}. */
export function formatCents(cents: Cents): string {
  const negative = cents < 0;
  // Math.abs before the split so -5 formats as "-0.05" rather than "-0.-5".
  const absolute = Math.abs(cents);
  const whole = Math.trunc(absolute / 100);
  const fraction = absolute % 100;
  const grouped = whole.toLocaleString("en-US");
  return `${negative ? "-" : ""}${grouped}.${String(fraction).padStart(2, "0")}`;
}

/** `123456` → `"$1,234.56"`. The one place a currency symbol is added. */
export function formatMoney(cents: Cents): string {
  const negative = cents < 0;
  const body = formatCents((negative ? -cents : cents) as Cents);
  return `${negative ? "-" : ""}$${body}`;
}

/**
 * Parse what a human typed into integer cents.
 *
 * Returns `null` rather than throwing: this runs on every keystroke of an
 * amount field, where "not a number yet" is the normal state and not an error.
 *
 * Accepts `$`, thousands separators, surrounding whitespace, a leading `-`,
 * and zero, one or two decimal places. Rejects anything with a third decimal —
 * silently rounding `1.005` to `1.00` would lose a user's money without
 * telling them, and there is no such thing as half a cent on a receipt.
 */
export function parseCents(input: string): Cents | null {
  const cleaned = input.trim().replace(/[$,\s]/g, "");
  if (cleaned === "" || cleaned === "-") {
    return null;
  }
  const match = /^(-?)(\d*)(?:\.(\d{0,2}))?$/.exec(cleaned);
  if (match === null) {
    return null;
  }
  const [, sign, whole, fraction = ""] = match;
  if (whole === "" && fraction === "") {
    return null;
  }
  // String padding rather than `* 100`: "0.07" * 100 is 7.000000000000001 in
  // IEEE 754, and Math.round would hide that here only to have it reappear
  // somewhere without a round.
  const cents = Number(`${whole === "" ? "0" : whole}${fraction.padEnd(2, "0")}`);
  if (!Number.isSafeInteger(cents)) {
    return null;
  }
  return (sign === "-" ? -cents : cents) as Cents;
}

/**
 * `83750` ppm → `"8.375%"`.
 *
 * Rates are parts-per-million integers for the same reason money is cents:
 * 9.4375% is exact as `94375` and inexact as a float.
 */
export function formatRatePpm(ppm: number): string {
  const whole = Math.trunc(ppm / 10_000);
  const fraction = String(ppm % 10_000).padStart(4, "0").replace(/0+$/, "");
  return fraction === "" ? `${whole}%` : `${whole}.${fraction}%`;
}

/**
 * The label for a receipt's tax line.
 *
 * Takes the rate that was *applied*, which is null when a receipt straddles
 * two — groceries at 3% and shampoo at 7.45% in one basket. There is no single
 * rate then, and printing either one labels the whole tax figure with a
 * percentage true of only part of it.
 *
 * The failing case this exists for: a $7.28 grocery subtotal in Highland owes
 * $0.22, and a line reading "Tax (7.45%) $0.22" makes correct arithmetic look
 * broken, because 7.45% of $7.28 is $0.54.
 */
export function taxLabel(appliedRatePpm: number | null): string {
  return appliedRatePpm === null ? "Tax (mixed rates)" : `Tax (${formatRatePpm(appliedRatePpm)})`;
}

/**
 * What a place charges, in one line: `"7.45% · groceries 3%"`.
 *
 * Names groceries explicitly rather than saying "food": the reduced rate is on
 * *unprepared* food, and a restaurant meal is taxed in full. A summary that
 * implied otherwise would have someone expecting the lower figure on a receipt
 * that will never show it.
 *
 * Lives here rather than in either view because both the first-run prompt and
 * the entry form answer the same question — what does this place charge — and
 * two copies would drift the first time one is reworded.
 */
export function rateSummary(taxRatePpm: number, foodRatePpm: number | null): string {
  const general = formatRatePpm(taxRatePpm);
  // Equal rates are not a reduction, and printing the same number twice reads
  // as a rule that does not exist.
  if (foodRatePpm === null || foodRatePpm >= taxRatePpm) return general;
  return `${general} · groceries ${formatRatePpm(foodRatePpm)}`;
}

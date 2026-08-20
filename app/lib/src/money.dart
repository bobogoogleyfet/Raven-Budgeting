/// Money in the frontend: display only.
///
/// # The rule this file exists to enforce
///
/// The core owns every monetary calculation. This layer receives integer
/// cents, formats them for a human, and parses what a human typed back into
/// integer cents so it can be sent straight back. It never adds, subtracts,
/// multiplies, apportions, or compares-with-tolerance.
///
/// That is not fussiness. Money logic written here has to be written again for
/// Android and again for iOS, and the three copies drift. `Money::allocate` in
/// the core already guarantees that split parts sum to the whole exactly;
/// re-deriving any of that here throws the guarantee away.
///
/// # How the type stops it
///
/// [Cents] is an extension type over `int` that does **not** implement `int`,
/// so none of `int`'s operators come with it: `a - b` on two `Cents` does not
/// compile at all. The TypeScript original could only make the *result*
/// unusable, because TS cannot forbid `-` on a number. Dart can, so this is
/// the stricter of the two — and it has to be, because Dart has no branded
/// types and a plain `int` alias would enforce nothing.
///
/// Ported from `ui/src/money.ts`. Where the two disagree, that file is the
/// original and this comment is a bug.
library;

/// An integer number of cents, as the core sends it.
///
/// Values arrive across the bridge as `PlatformInt64`, which is `int` on
/// native, so no decoding step is needed — only the wrapper, which exists
/// purely to make arithmetic a compile error.
extension type const Cents._(int _cents) {
  /// The underlying integer, for handing back to the core.
  int get value => _cents;

  bool get isNegative => _cents < 0;
}

const Cents zero = Cents._(0);

/// Trust an integer from the core as [Cents].
///
/// Call this at the bridge boundary and nowhere else. It is deliberately the
/// only way to mint a [Cents], so every value in the app is traceable to
/// something the core computed or to [parseCents].
///
/// The TypeScript version also had to check for a non-integer here, because a
/// float would have meant the i64-cents invariant broke on the Rust side. Dart
/// gets that for free: the bridge types the field as an integer, so a float
/// cannot arrive.
Cents fromCore(int value) => Cents._(value);

/// `123456` → `"1,234.56"`. No currency symbol; see [formatMoney].
String formatCents(Cents cents) {
  final int raw = cents.value;
  final bool negative = raw < 0;
  // Absolute value before the split, so -5 formats as "-0.05" rather than
  // "-0.-5".
  final int absolute = raw.abs();
  final int whole = absolute ~/ 100;
  final int fraction = absolute % 100;
  final String grouped = _group(whole);
  return '${negative ? '-' : ''}$grouped.${fraction.toString().padLeft(2, '0')}';
}

/// `123456` → `"$1,234.56"`. The one place a currency symbol is added.
///
/// The sign goes outside the symbol: `-$5.00`, never `$-5.00`. It belongs to
/// the amount, not to the currency.
String formatMoney(Cents cents) {
  final bool negative = cents.value < 0;
  final String body = formatCents(Cents._(cents.value.abs()));
  return '${negative ? '-' : ''}\$$body';
}

/// Comma-grouped thousands, by hand.
///
/// Not `NumberFormat`: it is display-only, its parser is unreliable for
/// currency, and it emits non-breaking spaces in some locales — which turns a
/// string comparison in a test into a puzzle. The rule here is one line long
/// and cannot surprise anyone.
String _group(int whole) {
  final String digits = whole.toString();
  final StringBuffer out = StringBuffer();
  for (int i = 0; i < digits.length; i++) {
    if (i > 0 && (digits.length - i) % 3 == 0) out.write(',');
    out.write(digits[i]);
  }
  return out.toString();
}

final RegExp _amount = RegExp(r'^(-?)(\d*)(?:\.(\d{0,2}))?$');

/// Parse what a human typed into integer cents.
///
/// Returns `null` rather than throwing: this runs on every keystroke of an
/// amount field, where "not a number yet" is the normal state and not an
/// error. `null` is not zero — a row with an unparseable amount is omitted
/// from what gets sent, never sent as `$0.00`.
///
/// Accepts `$`, thousands separators, surrounding whitespace, a leading `-`,
/// and zero, one or two decimal places. Rejects anything with a third decimal
/// — silently rounding `1.005` to `1.00` would lose a user's money without
/// telling them, and there is no such thing as half a cent on a receipt.
Cents? parseCents(String input) {
  final String cleaned = input.trim().replaceAll(RegExp(r'[$,\s]'), '');
  if (cleaned.isEmpty || cleaned == '-') return null;

  final RegExpMatch? match = _amount.firstMatch(cleaned);
  if (match == null) return null;

  final String sign = match.group(1) ?? '';
  final String whole = match.group(2) ?? '';
  final String fraction = match.group(3) ?? '';
  if (whole.isEmpty && fraction.isEmpty) return null;

  // String padding rather than `* 100`: "0.07" * 100 is 7.000000000000001 in
  // IEEE 754, and rounding would hide that here only to have it reappear
  // somewhere without a round.
  final int? parsed =
      int.tryParse('${whole.isEmpty ? '0' : whole}${fraction.padRight(2, '0')}');
  if (parsed == null) return null;

  return Cents._(sign == '-' ? -parsed : parsed);
}

/// `83750` ppm → `"8.375%"`.
///
/// Rates are parts-per-million integers for the same reason money is cents:
/// 9.4375% is exact as `94375` and inexact as a float.
String formatRatePpm(int ppm) {
  final int whole = ppm ~/ 10000;
  final String fraction =
      (ppm % 10000).toString().padLeft(4, '0').replaceAll(RegExp(r'0+$'), '');
  return fraction.isEmpty ? '$whole%' : '$whole.$fraction%';
}

/// What a place charges, in one line:
/// `"7.45% · groceries 3% · dining 8.45%"`.
///
/// Names groceries explicitly rather than saying "food": the reduced rate is
/// on *unprepared* food only. Prepared food goes the other way — a Utah county
/// may add a restaurant tax of up to 1% on top of the combined rate — so the
/// two are named separately and neither is called "food".
///
/// Each extra rate is dropped when it does not differ from the general one.
/// Equal rates are not a rule, and printing the same number three times reads
/// as a distinction that does not exist.
String rateSummary(int taxRatePpm, int? foodRatePpm, int? restaurantRatePpm) {
  final String general = formatRatePpm(taxRatePpm);
  final StringBuffer out = StringBuffer(general);
  if (foodRatePpm != null && foodRatePpm < taxRatePpm) {
    out.write(' · groceries ${formatRatePpm(foodRatePpm)}');
  }
  // Above, not below: a restaurant rate at or under the general rate is either
  // a county that levies no restaurant tax or two arguments swapped, and
  // neither is worth a line claiming dining is cheaper.
  if (restaurantRatePpm != null && restaurantRatePpm > taxRatePpm) {
    out.write(' · dining ${formatRatePpm(restaurantRatePpm)}');
  }
  return out.toString();
}

/// What a category row says under its picker.
///
/// A zero rate is not "taxed at nothing", it is "not sales-taxed at all" —
/// rent, a phone bill, a tank of fuel — and printing "0% sales tax" invites
/// the reader to wonder which rate failed to load.
String categoryRateLabel(int ppm) =>
    ppm == 0 ? 'no sales tax' : '${formatRatePpm(ppm)} sales tax';

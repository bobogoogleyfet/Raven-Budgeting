/// Rates as the setup and settings screens deal with them.
library;

/// `"7.75"` → `77500` ppm. Null if it is not a usable rate.
///
/// Deliberately not `double.tryParse`: that accepts `"7."`, `"1e2"`,
/// `"Infinity"` and, in some locales, `"7,5"`. A blank field must not become
/// 0%, which is exactly the state the first-run prompt exists to prevent.
int? parseRatePpm(String input) {
  final String cleaned =
      input.trim().replaceFirst(RegExp(r'%$'), '').trim();
  if (!RegExp(r'^\d*\.?\d+$').hasMatch(cleaned)) return null;

  final double? percent = double.tryParse(cleaned);
  if (percent == null || percent < 0 || percent > 100) return null;

  // Parts per million, so 9.4375% is exactly 94375 rather than a rounded
  // 9.44%. Rounding at the fourth decimal is far below a cent on any real
  // receipt and keeps the value an integer.
  return (percent * 10000).round();
}

/// The line naming which publication the rates came from.
///
/// Says "superseded" rather than "out of date" when the quarter has turned:
/// the figures are the state's last published ones, not junk, and the app is
/// still usable on them. What the user needs to know is that a newer set
/// exists and this machine has not managed to fetch it — which on a laptop
/// that has been offline is information, not an error.
String releaseLine({String? quarter, bool current = false}) {
  if (quarter == null) return '';
  final String source = "Rates from Utah's $quarter publication";
  return current
      ? '$source.'
      : '$source — superseded; a newer one has not downloaded yet.';
}

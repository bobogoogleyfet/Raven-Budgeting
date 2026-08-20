/// Dates in the frontend.
///
/// The core never reads the clock — every date-dependent function there takes
/// `today` explicitly — so knowing what day it is is this layer's job, and
/// getting it wrong files a receipt into the wrong day and, at a month
/// boundary, the wrong budget month.
///
/// Ported from `ui/src/dom.ts` and `ui/src/datefield.ts`.
library;

String _pad(int value, int width) => value.toString().padLeft(width, '0');

/// Today as `YYYY-MM-DD` in the user's own timezone.
///
/// Deliberately built from local getters rather than from
/// `DateTime.toIso8601String()`, which is what `toUtc()` feeds: at 8pm in Salt
/// Lake City that returns tomorrow's date.
String today([DateTime? now]) {
  final DateTime d = now ?? DateTime.now();
  return '${_pad(d.year, 4)}-${_pad(d.month, 2)}-${_pad(d.day, 2)}';
}

/// The current `YYYY-MM`, in the user's own timezone.
String thisMonth([DateTime? now]) => today(now).substring(0, 7);

/// Assemble typed segments into `YYYY-MM-DD`, or `null` if they are not a real
/// date.
///
/// Rejects the impossible rather than clamping it. Silently turning 2026-02-30
/// into 2026-02-28 would file a receipt on a day the user did not choose, and
/// on a month boundary that is the wrong budget month.
String? toIso(String month, String day, String year) {
  if (year.length != 4) return null;

  final int? y = int.tryParse(year);
  final int? m = int.tryParse(month);
  final int? d = int.tryParse(day);
  if (y == null || m == null || d == null) return null;
  if (y < 1 || m < 1 || m > 12 || d < 1 || d > 31) return null;

  // Round-tripping through DateTime is what catches February 30th and the leap
  // year rule without a table: the constructor rolls an out-of-range day into
  // the next month, so a mismatch means the date does not exist. Local, not
  // UTC — the same reason `today()` is.
  final DateTime candidate = DateTime(y, m, d);
  if (candidate.year != y || candidate.month != m || candidate.day != d) {
    return null;
  }

  return '${_pad(y, 4)}-${_pad(m, 2)}-${_pad(d, 2)}';
}

/// The three segments of a `YYYY-MM-DD`, or empty strings if it is not one.
({String month, String day, String year}) fromIso(String iso) {
  final RegExpMatch? match = RegExp(r'^(\d{4})-(\d{2})-(\d{2})$').firstMatch(iso);
  if (match == null) return (month: '', day: '', year: '');
  return (month: match.group(2)!, day: match.group(3)!, year: match.group(1)!);
}

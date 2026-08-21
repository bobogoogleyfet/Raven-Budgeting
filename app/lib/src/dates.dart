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

/// The month [count] months away from `YYYY-MM`, rolling the year.
///
/// `DateTime` does the rolling: December plus one is month 13, which the
/// constructor normalises to January of the next year. Doing that arithmetic by
/// hand is how a screen ends up offering `2026-13`, which is a month the core
/// will refuse and the reader cannot get out of.
String addMonths(String yearMonth, int count) {
  final List<String> parts = yearMonth.split('-');
  return thisMonth(DateTime(int.parse(parts[0]), int.parse(parts[1]) + count));
}

/// A month's first and last day, as the inclusive `YYYY-MM-DD` bounds a date
/// filter takes.
///
/// The end is day zero of the *next* month, which `DateTime` normalises to the
/// last day of this one — so February, and the leap year rule, are answered by
/// the calendar rather than by a table of month lengths copied into a screen.
/// Getting that end wrong drops the last receipt of the month out of a total
/// that still calls itself the month's.
({String from, String to}) monthRange(String yearMonth) {
  final List<String> parts = yearMonth.split('-');
  final int year = int.parse(parts[0]);
  final int month = int.parse(parts[1]);
  return (
    from: today(DateTime(year, month, 1)),
    to: today(DateTime(year, month + 1, 0)),
  );
}

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

const List<String> _monthNames = [
  'January', 'February', 'March', 'April', 'May', 'June',
  'July', 'August', 'September', 'October', 'November', 'December',
];

/// `"2026-08"` → `"August 2026"`. Falls back to the input when it is not a
/// month, because a screen that says `2026-13` is more use than one that says
/// nothing.
String monthLabel(String yearMonth) {
  final RegExpMatch? match = RegExp(r'^(\d{4})-(\d{2})$').firstMatch(yearMonth);
  if (match == null) return yearMonth;
  final int month = int.parse(match.group(2)!);
  if (month < 1 || month > 12) return yearMonth;
  return '${_monthNames[month - 1]} ${match.group(1)}';
}

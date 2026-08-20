import 'package:app/src/dates.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('today is local, not UTC', () {
    // 8pm in Salt Lake City on the last day of a month. Read as UTC this is
    // the 1st of the next month, which files a receipt into the wrong budget.
    final DateTime evening = DateTime(2026, 8, 31, 20, 30);
    expect(today(evening), '2026-08-31');
    expect(thisMonth(evening), '2026-08');
  });

  test('single digits are padded', () {
    expect(today(DateTime(2026, 1, 5)), '2026-01-05');
    expect(thisMonth(DateTime(2026, 1, 5)), '2026-01');
  });

  test('today with no argument is a well-formed date', () {
    expect(today(), matches(RegExp(r'^\d{4}-\d{2}-\d{2}$')));
    expect(thisMonth(), matches(RegExp(r'^\d{4}-\d{2}$')));
  });

  test('segments assemble into an ISO date', () {
    expect(toIso('08', '20', '2026'), '2026-08-20');
    expect(toIso('8', '5', '2026'), '2026-08-05');
  });

  test('an impossible date is refused rather than clamped', () {
    expect(toIso('02', '30', '2026'), isNull);
    expect(toIso('02', '29', '2025'), isNull);
    expect(toIso('13', '01', '2026'), isNull);
    expect(toIso('00', '01', '2026'), isNull);
    expect(toIso('01', '00', '2026'), isNull);
  });

  test('a leap day is a real date', () {
    expect(toIso('02', '29', '2024'), '2024-02-29');
  });

  test('an incomplete year is not a date yet', () {
    expect(toIso('01', '01', '202'), isNull);
    expect(toIso('01', '01', ''), isNull);
    expect(toIso('01', '01', 'abcd'), isNull);
  });

  test('an ISO date splits back into its segments', () {
    expect(fromIso('2026-08-20'), (month: '08', day: '20', year: '2026'));
  });

  test('junk splits into empty segments', () {
    expect(fromIso('nope'), (month: '', day: '', year: ''));
    expect(fromIso('2026-8-20'), (month: '', day: '', year: ''));
  });
}

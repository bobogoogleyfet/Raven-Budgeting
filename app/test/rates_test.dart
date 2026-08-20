import 'package:app/src/rates.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('a percentage becomes parts per million', () {
    expect(parseRatePpm('7.75'), 77500);
    expect(parseRatePpm('0'), 0);
    expect(parseRatePpm('10'), 100000);
  });

  test('a trailing percent sign and padding are accepted', () {
    expect(parseRatePpm(' 7.75% '), 77500);
    expect(parseRatePpm('7.75%'), 77500);
  });

  test('four decimal places survive, which is the point of ppm', () {
    // 9.4375% is the rate that percent-with-2-decimals rounds away.
    expect(parseRatePpm('9.4375'), 94375);
  });

  test('a rate outside 0-100% is refused', () {
    expect(parseRatePpm('-1'), isNull);
    expect(parseRatePpm('101'), isNull);
  });

  test('junk is refused rather than read as zero', () {
    // An empty field would otherwise silently configure a 0% rate — exactly
    // the state the first-run prompt exists to prevent.
    for (final input in ['', '   ', '%', 'abc', '7.7.5', 'seven', '7.', '1e2']) {
      expect(parseRatePpm(input), isNull, reason: 'input "$input"');
    }
  });

  test('the rates name the publication they came from', () {
    expect(
      releaseLine(quarter: '2026Q3', current: true),
      "Rates from Utah's 2026Q3 publication.",
    );
  });

  test('a superseded publication says so without calling itself broken', () {
    final line = releaseLine(quarter: '2026Q2', current: false);
    expect(line, contains('2026Q2'));
    expect(line, contains('superseded'));
  });

  test('no publication yet prints nothing rather than a blank claim', () {
    expect(releaseLine(), '');
  });
}

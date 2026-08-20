// Tests for the display/parse boundary.
//
// These matter more than their size suggests: this is the only Dart in the app
// allowed anywhere near a monetary value, so a bug here is a bug the Rust test
// suite structurally cannot catch.
//
// Ported case for case from `ui/src/money.test.ts`.

import 'package:app/src/money.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('formatting pads the cents rather than truncating them', () {
    expect(formatCents(fromCore(5)), '0.05');
    expect(formatCents(fromCore(50)), '0.50');
    expect(formatCents(fromCore(100)), '1.00');
    expect(formatCents(zero), '0.00');
  });

  test('formatting groups thousands and keeps the sign outside the symbol', () {
    expect(formatCents(fromCore(123456)), '1,234.56');
    expect(formatMoney(fromCore(123456)), r'$1,234.56');
    // "-$5.00", not "$-5.00": the sign belongs to the amount, not the currency.
    expect(formatMoney(fromCore(-500)), r'-$5.00');
    expect(formatCents(fromCore(-5)), '-0.05');
  });

  test('grouping holds past a million', () {
    expect(formatCents(fromCore(100000)), '1,000.00');
    expect(formatCents(fromCore(123456789)), '1,234,567.89');
    expect(formatMoney(fromCore(-123456789)), r'-$1,234,567.89');
  });

  test('parsing accepts what people actually type', () {
    expect(parseCents('12.34')?.value, 1234);
    expect(parseCents(r'$12.34')?.value, 1234);
    expect(parseCents(' 1,234.56 ')?.value, 123456);
    expect(parseCents('12')?.value, 1200);
    expect(parseCents('12.')?.value, 1200);
    expect(parseCents('.5')?.value, 50);
    expect(parseCents('0.07')?.value, 7);
    expect(parseCents('-5.00')?.value, -500);
  });

  test('a third decimal place is refused rather than silently rounded', () {
    expect(parseCents('1.005'), isNull);
    expect(parseCents('0.001'), isNull);
  });

  test('an incomplete or junk amount parses as null, not as zero', () {
    for (final input in ['', '   ', '-', 'abc', '1.2.3', r'$', '1a']) {
      expect(parseCents(input), isNull, reason: 'input "$input"');
    }
  });

  test('parsing is exact where floating point is not', () {
    expect(parseCents('0.07')?.value, 7);
    expect(parseCents('0.29')?.value, 29);
    expect(parseCents('1.10')?.value, 110);

    // Round-tripping every cent of the first two hundred dollars.
    for (int cents = 0; cents <= 20000; cents++) {
      final String text = formatCents(fromCore(cents));
      expect(parseCents(text)?.value, cents, reason: 'round trip failed at $cents');
    }
  });

  test('the round trip survives grouping and a sign', () {
    // The TypeScript loop stops at $200, so it never exercises the thousands
    // separator or a negative. Both are formatted here and parsed elsewhere.
    for (final int cents in [99999, 100000, 100001, 1234567, -1, -100000]) {
      expect(parseCents(formatCents(fromCore(cents)))?.value, cents);
      expect(parseCents(formatMoney(fromCore(cents)))?.value, cents);
    }
  });

  test('rates render exactly, including the awkward ones', () {
    expect(formatRatePpm(83750), '8.375%');
    expect(formatRatePpm(94375), '9.4375%');
    expect(formatRatePpm(70000), '7%');
    expect(formatRatePpm(0), '0%');
  });

  test('a reduced food rate is named alongside the general one', () {
    expect(rateSummary(74500, 30000), '7.45% · groceries 3%');
  });

  test('no reduced rate says nothing about groceries', () {
    expect(rateSummary(74500, null), '7.45%');
  });

  test('a food rate equal to the general one is not a reduction', () {
    expect(rateSummary(74500, 74500), '7.45%');
  });

  test('a category with no sales tax says so, rather than showing 0%', () {
    expect(categoryRateLabel(0), 'no sales tax');
    expect(categoryRateLabel(30000), '3% sales tax');
    expect(categoryRateLabel(74500), '7.45% sales tax');
  });
}

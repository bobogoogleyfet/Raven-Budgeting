import 'package:app/src/widgets/date_field.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// The three segments, in order: month, day, year.
Finder segment(int index) => find.byType(TextField).at(index);

Future<List<String>> pump(WidgetTester tester, {String value = ''}) async {
  final emitted = <String>[];
  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: DateField(value: value, onChanged: emitted.add),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return emitted;
}

/// Type into whichever segment has focus, the way a numpad would.
Future<void> typeDigits(WidgetTester tester, String digits) async {
  for (final digit in digits.split('')) {
    final focused = tester.widgetList<TextField>(find.byType(TextField));
    final field = focused.firstWhere((f) => f.focusNode?.hasFocus ?? false);
    final controller = field.controller!;
    await tester.enterText(
      find.byWidget(
        tester.widgetList<TextField>(find.byType(TextField)).firstWhere(
              (f) => identical(f.controller, controller),
            ),
      ),
      controller.text + digit,
    );
    await tester.pumpAndSettle();
  }
}

/// The field on a window [width] logical pixels wide, laid out with the
/// padding the entry form gives it, and every layout error that raised.
///
/// The handler is put back before this returns — a failed `expect` while it is
/// still overridden wedges the binding instead of reporting itself.
Future<List<String>> pumpAt(WidgetTester tester, double width) async {
  tester.view.physicalSize = Size(width, 800);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  final errors = <String>[];
  final previous = FlutterError.onError;
  FlutterError.onError = (details) => errors.add(details.exceptionAsString());

  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: Padding(
          padding: const EdgeInsets.all(16),
          child: DateField(value: '2026-08-20', onChanged: (_) {}),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();

  FlutterError.onError = previous;
  return errors;
}

void main() {
  testWidgets('an existing date fills the segments', (tester) async {
    await pump(tester, value: '2026-08-20');

    expect(find.widgetWithText(TextField, '08'), findsOneWidget);
    expect(find.widgetWithText(TextField, '20'), findsOneWidget);
    expect(find.widgetWithText(TextField, '2026'), findsOneWidget);
  });

  testWidgets('a full date is emitted as YYYY-MM-DD', (tester) async {
    final emitted = await pump(tester);

    await tester.enterText(segment(0), '08');
    await tester.enterText(segment(1), '20');
    await tester.enterText(segment(2), '2026');
    await tester.pumpAndSettle();

    expect(emitted.last, '2026-08-20');
  });

  testWidgets('a half-typed date emits nothing rather than a stale one',
      (tester) async {
    final emitted = await pump(tester);

    await tester.enterText(segment(0), '08');
    await tester.pumpAndSettle();

    // Keeping the last good date would preview a receipt against a day the
    // user is in the middle of changing.
    expect(emitted.last, '');
  });

  testWidgets('a month past 1 hands over without waiting for a second digit',
      (tester) async {
    await pump(tester);
    await tester.tap(segment(0));
    await tester.pumpAndSettle();

    // 8 can only be August, so waiting costs a keystroke on every date.
    await tester.enterText(segment(0), '8');
    await tester.pumpAndSettle();

    final day = tester.widget<TextField>(segment(1));
    expect(day.focusNode?.hasFocus, isTrue);
  });

  testWidgets('a month that could still take a digit waits', (tester) async {
    await pump(tester);
    await tester.tap(segment(0));
    await tester.pumpAndSettle();

    // 1 could be January, October, November or December.
    await tester.enterText(segment(0), '1');
    await tester.pumpAndSettle();

    expect(tester.widget<TextField>(segment(0)).focusNode?.hasFocus, isTrue);
  });

  testWidgets('two digits hand over even when a third was possible',
      (tester) async {
    await pump(tester);
    await tester.tap(segment(0));
    await tester.pumpAndSettle();

    await tester.enterText(segment(0), '12');
    await tester.pumpAndSettle();

    expect(tester.widget<TextField>(segment(1)).focusNode?.hasFocus, isTrue);
  });

  testWidgets('a day past 3 hands over to the year', (tester) async {
    await pump(tester);
    await tester.tap(segment(1));
    await tester.pumpAndSettle();

    await tester.enterText(segment(1), '4');
    await tester.pumpAndSettle();

    expect(tester.widget<TextField>(segment(2)).focusNode?.hasFocus, isTrue);
  });

  testWidgets('an impossible date is refused rather than clamped',
      (tester) async {
    final emitted = await pump(tester);

    await tester.enterText(segment(0), '02');
    await tester.enterText(segment(1), '30');
    await tester.enterText(segment(2), '2026');
    await tester.pumpAndSettle();

    // Silently turning the 30th of February into the 28th files a receipt on a
    // day the user did not choose.
    expect(emitted.last, '');
  });

  testWidgets('a leap day is a real date', (tester) async {
    final emitted = await pump(tester);

    await tester.enterText(segment(0), '02');
    await tester.enterText(segment(1), '29');
    await tester.enterText(segment(2), '2024');
    await tester.pumpAndSettle();

    expect(emitted.last, '2024-02-29');
  });

  testWidgets('letters never reach a segment', (tester) async {
    await pump(tester);

    await tester.enterText(segment(0), 'ab');
    await tester.pumpAndSettle();

    expect(tester.widget<TextField>(segment(0)).controller?.text, '');
  });

  // A phone held in one hand, and a slightly larger one.
  for (final width in [320.0, 360.0]) {
    testWidgets('the whole field fits a ${width.toInt()}px window',
        (tester) async {
      final errors = await pumpAt(tester, width);

      expect(errors, isEmpty);
    });

    testWidgets('the calendar button is not clipped at ${width.toInt()}px',
        (tester) async {
      await pumpAt(tester, width);

      // Whole, and still M3's minimum touch target. It is the only way into
      // the field for someone who is not typing, so it is the last thing that
      // may be trimmed to fit.
      final button = tester.getRect(find.byType(IconButton));
      expect(button.size, const Size(48, 48));
      expect(
        button.right,
        lessThanOrEqualTo(tester.getRect(find.byType(DateField)).right),
      );
    });

    testWidgets('the segments keep their width at ${width.toInt()}px',
        (tester) async {
      await pumpAt(tester, width);

      // Room was found between the boxes, not taken out of them: a segment
      // narrow enough to hide a digit would be a worse fit than the one that
      // overflowed.
      expect(tester.getSize(segment(0)).width, 46);
      expect(tester.getSize(segment(1)).width, 46);
      expect(tester.getSize(segment(2)).width, 68);
    });
  }
}

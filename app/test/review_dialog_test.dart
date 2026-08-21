import 'package:app/src/api.dart';
import 'package:app/src/money.dart';
import 'package:app/src/screens/review_dialog.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

PreviewRow row(int id, String name, int subtotal, int tax) => PreviewRow(
      categoryId: id,
      categoryName: name,
      subtotal: fromCore(subtotal),
      tax: fromCore(tax),
      amount: fromCore(subtotal + tax),
    );

Preview preview({
  List<PreviewRow> rows = const [],
  int subtotal = 0,
  int tax = 0,
  int total = 0,
}) =>
    Preview(
      subtotal: fromCore(subtotal),
      tax: fromCore(tax),
      total: fromCore(total),
      taxIsEstimated: true,
      taxRatePpm: 74500,
      appliedRatePpm: 74500,
      rows: rows,
      warnings: const [],
      blockers: const [],
    );

/// One category, the shape of nearly every receipt.
Preview groceries() => preview(
      rows: [row(1, 'Groceries', 728, 22)],
      subtotal: 728,
      tax: 22,
      total: 750,
    );

/// Opens the dialog on a window [width] logical pixels wide and returns every
/// layout error raised while it was laid out.
///
/// A row that does not fit fires one error per offending Row, so the answer is
/// a list rather than a flag: which rows overflowed is the useful part. The
/// handler is put back before this returns — a failed `expect` while it is
/// still overridden wedges the binding instead of reporting itself.
Future<List<String>> pump(
  WidgetTester tester, {
  required double width,
  Preview? initial,
}) async {
  final Preview start = initial ?? groceries();
  tester.view.physicalSize = Size(width, 800);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  final errors = <String>[];
  final previous = FlutterError.onError;
  FlutterError.onError = (details) => errors.add(details.exceptionAsString());

  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: Builder(
          builder: (context) => TextButton(
            onPressed: () => showReviewDialog(
              context: context,
              preview: start,
              recompute: (edits) async => start,
            ),
            child: const Text('open'),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('open'));
  await tester.pumpAndSettle();

  FlutterError.onError = previous;
  return errors;
}

void main() {
  // A phone held in one hand, and a slightly larger one. At 320 the dialog
  // surface is 240px wide and its content 192px; at 360, 232px. Two 110px
  // amount columns fit in neither with a category name still on the line.
  for (final width in [320.0, 360.0]) {
    final String at = 'at ${width.toInt()}px';

    testWidgets('lays the receipt out with nothing overflowing $at',
        (tester) async {
      final errors = await pump(tester, width: width);

      expect(find.text('Does this match your receipt?'), findsOneWidget);
      expect(errors, isEmpty);
    });

    testWidgets('keeps the category name readable $at', (tester) async {
      await pump(tester, width: width);

      // The failure this guards is the silent one: at 360 the fixed columns
      // cleared the edge by 4px, so nothing overflowed and the label they had
      // squeezed to nothing was simply not there to read.
      expect(tester.getSize(find.text('Groceries')).width, greaterThan(100));
    });

    testWidgets('keeps both figures editable $at', (tester) async {
      await pump(tester, width: width);

      // The amount and its tax, however the line is arranged. A narrow layout
      // that dropped one would lose the correction this dialog exists for.
      expect(find.byType(TextField), findsNWidgets(2));
      expect(find.text(r'$7.50'), findsOneWidget);
    });

    testWidgets('a three-category receipt still fits $at', (tester) async {
      final errors = await pump(
        tester,
        width: width,
        initial: preview(
          rows: [
            row(1, 'Groceries', 728, 22),
            row(2, 'Household goods', 1250, 93),
            row(3, 'Prepared food', 899, 90),
          ],
          subtotal: 2877,
          tax: 205,
          total: 3082,
        ),
      );

      expect(errors, isEmpty);
      expect(find.text('Household goods'), findsOneWidget);
    });
  }

  testWidgets('the amount and tax columns stay side by side when there is room',
      (tester) async {
    final errors = await pump(tester, width: 900);

    // Wide enough for the table, where the headings are what name the figures
    // and each line is one row.
    expect(find.text('Amount'), findsOneWidget);
    expect(find.text('Tax'), findsOneWidget);
    expect(errors, isEmpty);
  });

  testWidgets('the tax box is still the second one when the line is stacked',
      (tester) async {
    await pump(tester, width: 320);

    await tester.enterText(find.byType(TextField).last, '0.25');
    await tester.pumpAndSettle();

    expect(find.text('0.25'), findsOneWidget);
    // The subtotal box, first, is untouched — the two were not swapped by the
    // narrow layout.
    expect(
      tester.widget<TextField>(find.byType(TextField).first).controller?.text,
      '7.28',
    );
  });
}

import 'package:app/src/api.dart';
import 'package:app/src/money.dart';
import 'package:app/src/screens/entry_screen.dart';
import 'package:app/src/rust/api/entry.dart'
    show AreaDto, CategoryRateDto, ContextDto, LocationDto, MerchantDto;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

CategoryDto category(int id, String name) =>
    CategoryDto(id: id, name: name, groupName: 'Frequent', isBudgeted: true);

Preview preview({
  List<PreviewRow> rows = const [],
  List<String> blockers = const [],
  List<String> warnings = const [],
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
      warnings: warnings,
      blockers: blockers,
    );

PreviewRow row(int id, String name, int subtotal, int tax) => PreviewRow(
      categoryId: id,
      categoryName: name,
      subtotal: fromCore(subtotal),
      tax: fromCore(tax),
      amount: fromCore(subtotal + tax),
    );

/// Stands in for the core.
class Fake {
  Fake({this.previewResult, this.saveId = 7});

  final List<SubtotalDraft> previews = [];
  final List<SubtotalDraft> saves = [];
  final List<String?> contexts = [];
  final List<String> ensured = [];

  Preview Function(SubtotalDraft draft)? previewResult;
  int saveId;
  Object? previewError;
  Object? saveError;

  EntryApi get api => EntryApi(
        listCategories: () async => [
          category(1, 'Groceries'),
          category(2, 'Gas'),
          category(3, 'Household'),
        ],
        listMerchants: () async =>
            [MerchantDto(id: 1, name: "Macey's", defaultCategoryId: null)],
        listLocations: () async =>
            [LocationDto(id: 1, name: 'Provo', taxRatePpm: 74500, isHome: true)],
        listAreas: () async => [
          AreaDto(
            code: '25-090',
            name: 'Orem',
            taxRatePpm: 72500,
            foodRatePpm: 30000,
          ),
        ],
        entryContext: (merchant) async {
          contexts.add(merchant);
          return ContextDto(
            merchantName: merchant,
            locationName: 'Provo',
            taxRatePpm: 74500,
            foodRatePpm: 30000,
            categoryRates: [
              CategoryRateDto(categoryId: 1, ratePpm: 30000),
              CategoryRateDto(categoryId: 2, ratePpm: 0),
              CategoryRateDto(categoryId: 3, ratePpm: 74500),
            ],
            defaultCategoryId: 1,
            detailLevel: 'grouped',
            recentMerchants: const ["Macey's", 'Chevron'],
          );
        },
        ensureLocation: (name) async {
          ensured.add(name);
          return LocationDto(
              id: 2, name: name, taxRatePpm: 72500, isHome: false);
        },
        previewSubtotals: (draft) async {
          previews.add(draft);
          if (previewError != null) throw previewError!;
          return previewResult?.call(draft) ?? preview();
        },
        saveSubtotals: (draft) async {
          saves.add(draft);
          if (saveError != null) throw saveError!;
          return saveId;
        },
      );
}

Future<void> pump(WidgetTester tester, Fake fake) async {
  tester.view.physicalSize = const Size(900, 1000);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    MaterialApp(home: Scaffold(body: EntryScreen(core: fake.api))),
  );
  await tester.pumpAndSettle();
}

/// Type into the one amount field of the first row.
Future<void> enterAmount(WidgetTester tester, String text) async {
  await tester.enterText(find.widgetWithText(TextField, '0.00').first, text);
  // Past the debounce: the preview waits 150ms so typing "154.32" is one round
  // trip rather than six.
  await tester.pump(const Duration(milliseconds: 200));
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('opens with today, one row, and the store chips', (tester) async {
    final fake = Fake();
    await pump(tester, fake);

    expect(find.text('Store'), findsOneWidget);
    expect(find.text('Categories'), findsOneWidget);
    // One blank row to start, and no way to remove the only one.
    expect(find.widgetWithText(TextField, '0.00'), findsOneWidget);
    expect(find.byTooltip('Remove this category'), findsNothing);
    expect(find.text("Macey's"), findsWidgets);
    expect(fake.contexts, [null]);
  });

  testWidgets('a row still being typed is left out, not sent as zero',
      (tester) async {
    final fake = Fake();
    await pump(tester, fake);

    await enterAmount(tester, 'not a number');

    // No preview at all: there is nothing to ask about. A $0.00 row would
    // claim the user meant to file nothing under that category.
    expect(fake.previews, isEmpty);
  });

  testWidgets('a parseable amount is sent as cents', (tester) async {
    final fake = Fake();
    await pump(tester, fake);

    await enterAmount(tester, '7.28');

    expect(fake.previews, isNotEmpty);
    expect(fake.previews.last.rows.single.subtotal.value, 728);
    // The form never sends a tax of its own; that is the rate's job.
    expect(fake.previews.last.rows.single.tax, isNull);
  });

  testWidgets('the date goes as YYYY-MM-DD', (tester) async {
    final fake = Fake();
    await pump(tester, fake);

    await enterAmount(tester, '10.00');

    expect(fake.previews.last.txnDate, matches(RegExp(r'^\d{4}-\d{2}-\d{2}$')));
  });

  testWidgets('adding and removing rows', (tester) async {
    final fake = Fake();
    await pump(tester, fake);

    await tester.tap(find.text('Add category'));
    await tester.pumpAndSettle();

    expect(find.widgetWithText(TextField, '0.00'), findsNWidgets(2));
    // Now that there are two, either can go.
    expect(find.byTooltip('Remove this category'), findsNWidgets(2));

    await tester.tap(find.byTooltip('Remove this category').first);
    await tester.pumpAndSettle();

    expect(find.widgetWithText(TextField, '0.00'), findsOneWidget);
    expect(find.byTooltip('Remove this category'), findsNothing);
  });

  testWidgets('blockers show and block the save', (tester) async {
    final fake = Fake()
      ..previewResult = (draft) => preview(blockers: const ['row 1 has no category']);
    await pump(tester, fake);

    await enterAmount(tester, '7.28');

    expect(find.text('row 1 has no category'), findsOneWidget);
    final button = tester.widget<FilledButton>(
      find.widgetWithText(FilledButton, 'Save expense'),
    );
    expect(button.onPressed, isNull);
  });

  testWidgets('warnings show but do not block', (tester) async {
    final fake = Fake()
      ..previewResult = (draft) => preview(
            rows: [row(1, 'Groceries', 728, 22)],
            warnings: const ['row 1 is negative (-\$5.00), which records a refund'],
          );
    await pump(tester, fake);

    await enterAmount(tester, '7.28');

    expect(find.textContaining('records a refund'), findsOneWidget);
    final button = tester.widget<FilledButton>(
      find.widgetWithText(FilledButton, 'Save expense'),
    );
    expect(button.onPressed, isNotNull);
  });

  testWidgets('an empty form cannot be saved', (tester) async {
    final fake = Fake();
    await pump(tester, fake);

    final button = tester.widget<FilledButton>(
      find.widgetWithText(FilledButton, 'Save expense'),
    );
    expect(button.onPressed, isNull);
  });

  testWidgets('saving asks first, and writes what was approved',
      (tester) async {
    final fake = Fake()
      ..previewResult = (draft) => preview(
            rows: [row(1, 'Groceries', 728, 22)],
            subtotal: 728,
            tax: 22,
            total: 750,
          );
    await pump(tester, fake);
    await enterAmount(tester, '7.28');

    await tester.tap(find.widgetWithText(FilledButton, 'Save expense'));
    await tester.pumpAndSettle();

    // $7.28 of groceries owes $0.22, not $0.53 — the reduced rate on
    // unprepared food, worked out by the core.
    expect(find.text('Does this match your receipt?'), findsOneWidget);
    expect(find.text(r'$7.50'), findsOneWidget);
    expect(fake.saves, isEmpty);

    await tester.tap(find.widgetWithText(FilledButton, 'Save Expense'));
    await tester.pumpAndSettle();

    expect(fake.saves, hasLength(1));
    // Untouched, so the tax goes as null — "work it out from the rate". Were
    // it sent back, every save would claim to have been read off the paper and
    // `taxIsEstimated` would stop meaning anything.
    expect(fake.saves.single.rows.single.tax, isNull);
    expect(find.text('Saved expense #7.'), findsOneWidget);
  });

  testWidgets('an untouched tax is sent as null, a corrected one as cents',
      (tester) async {
    final fake = Fake()
      ..previewResult = (draft) => preview(
            rows: [row(1, 'Groceries', 728, 22)],
            subtotal: 728,
            tax: 22,
            total: 750,
          );
    await pump(tester, fake);
    await enterAmount(tester, '7.28');

    await tester.tap(find.widgetWithText(FilledButton, 'Save expense'));
    await tester.pumpAndSettle();

    // The tax box is the second amount field in the dialog's row.
    final taxField = find.byType(TextField).last;
    await tester.enterText(taxField, '0.25');
    await tester.pumpAndSettle();

    await tester.tap(find.widgetWithText(FilledButton, 'Save Expense'));
    await tester.pumpAndSettle();

    // Only a figure the user actually touched is sent as an override, or every
    // save would claim to have been read off the receipt.
    expect(fake.saves.single.rows.single.tax?.value, 25);
  });

  testWidgets('backing out of the dialog writes nothing and keeps the form',
      (tester) async {
    final fake = Fake()
      ..previewResult = (draft) => preview(
            rows: [row(1, 'Groceries', 728, 22)],
            subtotal: 728,
            tax: 22,
            total: 750,
          );
    await pump(tester, fake);
    await enterAmount(tester, '7.28');

    await tester.tap(find.widgetWithText(FilledButton, 'Save expense'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(TextButton, 'Back'));
    await tester.pumpAndSettle();

    expect(fake.saves, isEmpty);
    // Nothing cleared: the receipt is intact for another go.
    expect(find.widgetWithText(TextField, '7.28'), findsOneWidget);
  });

  testWidgets('a saved form is blank, not a re-run of the last one',
      (tester) async {
    final fake = Fake()
      ..previewResult = (draft) => preview(
            rows: [row(1, 'Groceries', 728, 22)],
            subtotal: 728,
            tax: 22,
            total: 750,
          );
    await pump(tester, fake);
    await enterAmount(tester, '7.28');
    await tester.tap(find.widgetWithText(FilledButton, 'Save expense'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'Save Expense'));
    await tester.pumpAndSettle();

    // Leaving the amounts on screen invites a second click that files the same
    // expense twice.
    expect(find.widgetWithText(TextField, '7.28'), findsNothing);
    expect(find.widgetWithText(TextField, '0.00'), findsOneWidget);
    // And the context is re-read, because this store was just shopped at.
    expect(fake.contexts.length, greaterThan(1));
  });

  testWidgets('a failed save says so and keeps the receipt', (tester) async {
    final fake = Fake()
      ..previewResult = ((draft) => preview(
            rows: [row(1, 'Groceries', 728, 22)],
            subtotal: 728,
            tax: 22,
            total: 750,
          ))
      ..saveError = Exception('not found: location `Nowhere`');
    await pump(tester, fake);
    await enterAmount(tester, '7.28');
    await tester.tap(find.widgetWithText(FilledButton, 'Save expense'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'Save Expense'));
    await tester.pumpAndSettle();

    expect(find.textContaining('not found: location'), findsOneWidget);
    expect(find.widgetWithText(TextField, '7.28'), findsOneWidget);
  });
}

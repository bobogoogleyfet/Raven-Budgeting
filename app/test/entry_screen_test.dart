import 'package:app/src/api.dart';
import 'package:app/src/money.dart';
import 'package:app/src/screens/entry_screen.dart';
import 'package:app/src/rust/api/entry.dart'
    show AreaDto, CategoryRateDto, ContextDto, LocationDto, MerchantDto;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
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

    await tester.tap(find.widgetWithText(FilledButton, 'Save & New'));
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

    await tester.tap(find.widgetWithText(FilledButton, 'Save & New'));
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
    await tester.tap(find.widgetWithText(FilledButton, 'Save & New'));
    await tester.pumpAndSettle();

    // Leaving the amounts on screen invites a second click that files the same
    // expense twice.
    expect(find.widgetWithText(TextField, '7.28'), findsNothing);
    expect(find.widgetWithText(TextField, '0.00'), findsOneWidget);
    // And the context is re-read, because this store was just shopped at.
    expect(fake.contexts.length, greaterThan(1));
  });

  testWidgets('the dialog offers both destinations, and neither is "OK"',
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

    // Somebody working through a pile of receipts and somebody entering the one
    // in their pocket want opposite things, and the dialog is where that is
    // already being decided.
    expect(find.widgetWithText(TextButton, 'Save & Home'), findsOneWidget);
    expect(find.widgetWithText(FilledButton, 'Save & New'), findsOneWidget);
    expect(find.widgetWithText(TextButton, 'Back'), findsOneWidget);
    // Short because they are read at a glance. Not to keep them on one row:
    // below ~400px the OverflowBar stacks all three whatever their length,
    // which is M3's own fallback and is fine.
    for (final label in ['Save & Home', 'Save & New']) {
      expect(label.length, lessThanOrEqualTo(12), reason: '"$label" is a button');
    }
  });

  testWidgets('Save & New leaves the date focused, ready for the next receipt',
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
    await tester.tap(find.widgetWithText(FilledButton, 'Save & New'));
    await tester.pumpAndSettle();

    // A cleared form with the cursor nowhere makes the next receipt start with
    // a hunt for the first field. `autofocus` cannot do this: it fires when the
    // field appears, and the form is cleared in place rather than rebuilt.
    final EditableText month = tester.widget(
      find.byType(EditableText).first,
    );
    expect(month.focusNode.hasFocus, isTrue,
        reason: 'the date should be focused again after saving');
    // Focusing a segment selects it whole, so the date is typed over rather
    // than appended to.
    expect(month.controller.selection.baseOffset, 0);
    expect(
      month.controller.selection.extentOffset,
      month.controller.text.length,
    );
  });

  testWidgets('Save & Home closes the form and asks to go home',
      (tester) async {
    final fake = Fake()
      ..previewResult = (draft) => preview(
            rows: [row(1, 'Groceries', 728, 22)],
            subtotal: 728,
            tax: 22,
            total: 750,
          );
    tester.view.physicalSize = const Size(900, 1000);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);

    // Pushed the way the app pushes it: a full-screen dialog over something
    // else, which is why the form pops a result instead of navigating itself.
    Object? popped = 'not popped';
    var refreshed = 0;
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Builder(
            builder: (context) => TextButton(
              onPressed: () async {
                popped = await Navigator.of(context).push<bool>(
                  MaterialPageRoute<bool>(
                    fullscreenDialog: true,
                    builder: (_) => Scaffold(
                      body: EntryScreen(
                        core: fake.api,
                        onSaved: () => refreshed++,
                      ),
                    ),
                  ),
                );
              },
              child: const Text('open'),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();

    await enterAmount(tester, '7.28');
    await tester.tap(find.widgetWithText(FilledButton, 'Save expense'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(TextButton, 'Save & Home'));
    await tester.pumpAndSettle();

    expect(fake.saves.length, 1, reason: 'the expense is still written');
    expect(find.byType(EntryScreen), findsNothing, reason: 'the form closed');
    // `true` and not just "popped": dismissing any other way must leave the
    // user where they were.
    expect(popped, isTrue);
    // Home is where this goes, so Home has to hear about the new expense —
    // leaving must not skip the refresh that "Save & New" gets.
    expect(refreshed, 1, reason: 'the screens underneath are told to redraw');
  });

  testWidgets('a category cannot be invented by typing it into the row',
      (tester) async {
    final fake = Fake();
    await pump(tester, fake);

    // The field used to offer "Press Enter to add “Vet”" while onCommit quietly
    // matched nothing — so the prompt promised a category, none was made, and
    // the row was left with no category at all.
    final category = find.widgetWithText(TextField, 'Select a category');
    expect(category, findsOneWidget);
    // Tapped first: the suggestion list is an overlay that opens on focus, and
    // enterText alone does not raise it.
    await tester.tap(category);
    await tester.pumpAndSettle();
    await tester.enterText(category, 'Vet');
    await tester.pumpAndSettle();

    expect(find.textContaining('Press Enter to add'), findsNothing,
        reason: 'the form must not offer to create a category');
    // Refusing without saying why reads as a bug, so it names the fix.
    expect(find.textContaining('No category called'), findsOneWidget);
    expect(find.textContaining('Budget screen'), findsOneWidget);

    // Enter does not take it either: a strict field keeps the last name it
    // knew rather than one it does not.
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(find.text('Vet'), findsNothing);
  });

  testWidgets('a known category is still picked normally', (tester) async {
    // The guard must not cost the ordinary case: the whole list is still one
    // keystroke away.
    final fake = Fake();
    await pump(tester, fake);

    final category = find.widgetWithText(TextField, 'Select a category');
    await tester.tap(category);
    await tester.pumpAndSettle();
    await tester.enterText(category, 'Groc');
    await tester.pumpAndSettle();
    expect(find.text('Groceries'), findsWidgets);

    await tester.tap(find.text('Groceries').last);
    await tester.pumpAndSettle();
    expect(find.widgetWithText(TextField, 'Groceries'), findsOneWidget);
  });

  testWidgets('a bank amount carries its own tax, so none is added',
      (tester) async {
    // A statement gives one number per charge with the tax already inside it.
    // Adding a rate on top invents money nobody was charged.
    final fake = Fake();
    await pump(tester, fake);

    await tester.tap(find.widgetWithText(SegmentedButton<bool>, 'Bank'));
    await tester.pumpAndSettle();
    await enterAmount(tester, '13.59');

    final sent = fake.previews.last.rows.single;
    expect(sent.subtotal.value, 1359);
    // Zero, not null: null means "work it out from the rate", which is the
    // behaviour being switched off.
    expect(sent.tax?.value, 0,
        reason: 'a statement figure asserts there is nothing to add');
  });

  testWidgets('a receipt amount still leaves the tax to the rate',
      (tester) async {
    final fake = Fake();
    await pump(tester, fake);
    await enterAmount(tester, '13.59');

    final sent = fake.previews.last.rows.single;
    expect(sent.subtotal.value, 1359);
    expect(sent.tax, isNull, reason: 'Receipt is still the default');
  });

  testWidgets('switching source re-previews rather than leaving stale tax',
      (tester) async {
    final fake = Fake();
    await pump(tester, fake);
    await enterAmount(tester, '13.59');
    final int before = fake.previews.length;

    await tester.tap(find.widgetWithText(SegmentedButton<bool>, 'Bank'));
    await tester.pumpAndSettle();

    expect(fake.previews.length, greaterThan(before),
        reason: 'every amount on screen just changed meaning');
    expect(fake.previews.last.rows.single.tax?.value, 0);
  });

  testWidgets('each save notifies the caller, so the app can reload behind it',
      (tester) async {
    // What the shell hangs its refresh on. Every screen reads its data once in
    // initState, so without this a receipt is filed and the list behind the
    // form still shows the state from before it.
    final fake = Fake()
      ..previewResult = (draft) => preview(
            rows: [row(1, 'Groceries', 728, 22)],
            subtotal: 728,
            tax: 22,
            total: 750,
          );
    int saves = 0;
    tester.view.physicalSize = const Size(900, 1000);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: EntryScreen(core: fake.api, onSaved: () => saves++),
        ),
      ),
    );
    await tester.pumpAndSettle();

    await enterAmount(tester, '7.28');
    await tester.tap(find.widgetWithText(FilledButton, 'Save expense'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'Save & New'));
    await tester.pumpAndSettle();
    expect(saves, 1);

    // Fired per save, not per close: "Save & New" files several without ever
    // leaving the form.
    await enterAmount(tester, '3.50');
    await tester.tap(find.widgetWithText(FilledButton, 'Save expense'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'Save & New'));
    await tester.pumpAndSettle();
    expect(saves, 2);
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
    await tester.tap(find.widgetWithText(FilledButton, 'Save & New'));
    await tester.pumpAndSettle();

    expect(find.textContaining('not found: location'), findsOneWidget);
    expect(find.widgetWithText(TextField, '7.28'), findsOneWidget);
  });

  testWidgets('an amount fills from the cents up', (tester) async {
    final fake = Fake();
    await pump(tester, fake);

    // Typing 50 is fifty cents, not fifty dollars: nobody entering a receipt
    // types a decimal point.
    await enterAmount(tester, '50');
    expect(fake.previews.last.rows.single.subtotal.value, 50);

    await enterAmount(tester, '150');
    expect(fake.previews.last.rows.single.subtotal.value, 150);

    // And what is on screen is what was read back.
    expect(find.widgetWithText(TextField, '1.50'), findsOneWidget);
  });

  testWidgets('the amount field says what it wants, not an example',
      (tester) async {
    final fake = Fake();
    await pump(tester, fake);

    final field = tester.widget<TextField>(
      find.widgetWithText(TextField, '0.00').first,
    );
    expect(field.decoration?.hintText, '0.00');
  });

  testWidgets('the form opens with the date ready to type over', (tester) async {
    final fake = Fake();
    await pump(tester, fake);

    // The first thing a receipt needs is its date, and it is already filled
    // in — so it opens selected, to be typed over or passed through.
    final month = tester.widget<TextField>(find.byType(TextField).first);
    expect(month.focusNode?.hasFocus, isTrue);
    expect(month.controller?.selection.start, 0);
    expect(month.controller?.selection.end, month.controller?.text.length);
  });

  testWidgets('Enter walks the date and then the form', (tester) async {
    final fake = Fake();
    await pump(tester, fake);

    Future<void> enter() async {
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
    }

    // Month, day, year, and then out of the date and on to the store.
    await enter();
    expect(
      tester.widget<TextField>(find.byType(TextField).at(1)).focusNode?.hasFocus,
      isTrue,
      reason: 'day',
    );
    await enter();
    expect(
      tester.widget<TextField>(find.byType(TextField).at(2)).focusNode?.hasFocus,
      isTrue,
      reason: 'year',
    );
    await enter();
    expect(
      tester.widget<TextField>(find.byType(TextField).at(3)).focusNode?.hasFocus,
      isTrue,
      reason: 'the store field is next',
    );
  });
}

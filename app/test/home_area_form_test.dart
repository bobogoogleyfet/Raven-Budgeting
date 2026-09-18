import 'package:app/src/api.dart';
import 'package:app/src/screens/home_area_form.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

class Saved {
  Saved(
    this.name,
    this.areaCode,
    this.taxRatePpm,
    this.foodRatePpm,
    this.restaurantRatePpm,
  );

  final String name;
  final String? areaCode;
  final int taxRatePpm;
  final int? foodRatePpm;
  final int? restaurantRatePpm;
}

Future<List<Saved>> pump(
  WidgetTester tester, {
  bool firstRun = false,
  RateReleaseDto? release,
  RateReleaseDto? afterRefresh,
  RefreshDto? refreshResult,
  Object? refreshError,
  Object? saveError,
  bool canRefresh = true,
}) async {
  var refreshed = false;
  final saved = <Saved>[];
  tester.view.physicalSize = const Size(900, 1000);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: HomeAreaForm(
          firstRun: firstRun,
          canRefresh: canRefresh,
          submitLabel: 'Save',
          onSaved: () {},
          loadAreas: () async => [
            AreaDto(
              code: '25-090',
              name: 'Provo',
              taxRatePpm: 74500,
              foodRatePpm: 30000,
              restaurantRatePpm: 84500,
            ),
            AreaDto(
              code: '25-100',
              name: 'Orem',
              taxRatePpm: 72500,
              foodRatePpm: 30000,
              restaurantRatePpm: 82500,
            ),
          ],
          loadRelease: () async => refreshed ? (afterRefresh ?? release) : release,
          refresh: () async {
            refreshed = true;
            if (refreshError != null) throw refreshError;
            return refreshResult ??
                RefreshDto(
                  attempted: false,
                  installed: false,
                  areas: 0,
                  locationsUpdated: 0,
                  problem: null,
                );
          },
          save: ({
            required String name,
            required String? areaCode,
            required int taxRatePpm,
            required int? foodRatePpm,
            required int? restaurantRatePpm,
          }) async {
            if (saveError != null) throw saveError;
            saved.add(
              Saved(name, areaCode, taxRatePpm, foodRatePpm, restaurantRatePpm),
            );
          },
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return saved;
}

Future<void> chooseArea(WidgetTester tester, String name) async {
  await tester.enterText(find.byType(TextField).first, name);
  await tester.pumpAndSettle();
  await tester.testTextInput.receiveAction(TextInputAction.done);
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('a known area brings its rate, named as an estimate on first run',
      (tester) async {
    await pump(tester, firstRun: true);
    await chooseArea(tester, 'Provo');

    // Groceries at the reduced rate, named explicitly: the reduced rate is on
    // unprepared food, and a restaurant meal is taxed in full.
    expect(find.textContaining('7.45% · groceries 3%'), findsOneWidget);
    expect(find.textContaining("a receipt's own tax always wins"), findsOneWidget);
    expect(find.textContaining("Not in Utah's list"), findsNothing);
  });

  testWidgets('a known area saves its code and its food rate', (tester) async {
    final saved = await pump(tester);
    await chooseArea(tester, 'Provo');
    await tester.tap(find.widgetWithText(FilledButton, 'Save'));
    await tester.pumpAndSettle();

    expect(saved, hasLength(1));
    expect(saved.single.areaCode, '25-090');
    expect(saved.single.taxRatePpm, 74500);
    expect(saved.single.foodRatePpm, 30000);
  });

  testWidgets('an area the list does not know asks for the rate',
      (tester) async {
    await pump(tester);
    await chooseArea(tester, 'Boise');

    expect(find.textContaining("Not in Utah's list"), findsOneWidget);
  });

  testWidgets('a hand-typed rate carries no jurisdiction and no food rate',
      (tester) async {
    final saved = await pump(tester);
    await chooseArea(tester, 'Boise');

    await tester.enterText(find.byType(TextField).last, '6');
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'Save'));
    await tester.pumpAndSettle();

    expect(saved.single.name, 'Boise');
    expect(saved.single.taxRatePpm, 60000);
    // The user moved the choice off the list, so there is no jurisdiction to
    // follow — and carrying Utah's 3% to Boise would undertax every grocery
    // receipt entered there.
    expect(saved.single.areaCode, isNull);
    expect(saved.single.foodRatePpm, isNull);
  });

  testWidgets('a blank rate is refused rather than read as zero',
      (tester) async {
    final saved = await pump(tester);
    await chooseArea(tester, 'Boise');
    await tester.tap(find.widgetWithText(FilledButton, 'Save'));
    await tester.pumpAndSettle();

    expect(saved, isEmpty);
    expect(find.textContaining('Enter the sales-tax rate'), findsOneWidget);
  });

  testWidgets('an empty area name asks for one', (tester) async {
    final saved = await pump(tester);
    await tester.tap(find.widgetWithText(FilledButton, 'Save'));
    await tester.pumpAndSettle();

    expect(saved, isEmpty);
    expect(find.textContaining('Pick the area you shop in most'), findsOneWidget);
  });

  testWidgets('the publication is named, and says when it is superseded',
      (tester) async {
    await pump(
      tester,
      release: RateReleaseDto(
        quarter: '2026Q2',
        effectiveOn: '2026-04-01',
        current: false,
      ),
    );

    expect(find.textContaining('2026Q2'), findsOneWidget);
    expect(find.textContaining('superseded'), findsOneWidget);
  });

  testWidgets('no publication prints nothing rather than a blank claim',
      (tester) async {
    await pump(tester);

    expect(find.textContaining('Rates from'), findsNothing);
  });

  testWidgets('a refused save says why and stays put', (tester) async {
    final saved = await pump(
      tester,
      saveError: Exception('a reduced food rate cannot be above the general rate'),
    );
    await chooseArea(tester, 'Provo');
    await tester.tap(find.widgetWithText(FilledButton, 'Save'));
    await tester.pumpAndSettle();

    expect(saved, isEmpty);
    expect(find.textContaining('reduced food rate'), findsOneWidget);
  });

  testWidgets('checking for new rates installs one and renames the quarter',
      (tester) async {
    await pump(
      tester,
      release: RateReleaseDto(
        quarter: '2026Q2',
        effectiveOn: '2026-04-01',
        current: false,
      ),
      afterRefresh: RateReleaseDto(
        quarter: '2026Q3',
        effectiveOn: '2026-07-01',
        current: true,
      ),
      refreshResult: RefreshDto(
        attempted: true,
        installed: true,
        areas: 389,
        locationsUpdated: 2,
        problem: null,
      ),
    );

    expect(find.textContaining('superseded'), findsOneWidget);

    await tester.tap(find.text('Check for new rates'));
    await tester.pumpAndSettle();

    expect(find.textContaining('389 jurisdictions'), findsOneWidget);
    expect(find.textContaining("2026Q3 publication."), findsOneWidget);
    expect(find.textContaining('superseded'), findsNothing);
  });

  testWidgets('being offline says so without calling itself broken',
      (tester) async {
    await pump(
      tester,
      release: RateReleaseDto(
        quarter: '2026Q2',
        effectiveOn: '2026-04-01',
        current: false,
      ),
      refreshResult: RefreshDto(
        attempted: true,
        installed: false,
        areas: 0,
        locationsUpdated: 0,
        problem: 'dns error: failed to lookup address',
      ),
    );

    await tester.tap(find.text('Check for new rates'));
    await tester.pumpAndSettle();

    // The rates already installed are still the state's figures, and the app
    // still works on them.
    expect(find.textContaining('Could not fetch it'), findsOneWidget);
    expect(find.textContaining('superseded'), findsOneWidget);
  });

  testWidgets('a browser offers no rate check it could never complete',
      (tester) async {
    // The page may talk to its own origin and nothing else, so the Tax
    // Commission is out of reach — a button that could only ever fail is
    // worse than none. Where the rates came from is still said.
    await pump(
      tester,
      canRefresh: false,
      release: RateReleaseDto(
        quarter: '2026Q2',
        effectiveOn: '2026-04-01',
        current: true,
      ),
    );

    expect(find.text('Check for new rates'), findsNothing);
    expect(find.textContaining('2026Q2 publication.'), findsOneWidget);
  });

  testWidgets('a table that is already current says so', (tester) async {
    await pump(
      tester,
      release: RateReleaseDto(
        quarter: '2026Q3',
        effectiveOn: '2026-07-01',
        current: true,
      ),
    );

    await tester.tap(find.text('Check for new rates'));
    await tester.pumpAndSettle();

    expect(find.text('Already up to date.'), findsOneWidget);
  });
}

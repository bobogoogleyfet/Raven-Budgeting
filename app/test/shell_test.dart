import 'package:app/src/destinations.dart';
import 'package:app/src/shell.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// The navigation component is a function of the window size class, so the
/// test sets a window size and asks which one appeared. The breakpoints are
/// Material 3's; these widths sit either side of them.
Future<void> pumpAtWidth(WidgetTester tester, double width) async {
  tester.view.physicalSize = Size(width, 800);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    MaterialApp(
      home: AppShell(
        homeTitle: 'New expense',
        homeBuilder: (context) => const Text('screen:home'),
        screenBuilder: (context, destination) =>
            Text('screen:${destination.label}'),
      ),
    ),
  );
}

void main() {
  testWidgets('opens on the entry form, not on a destination', (tester) async {
    await pumpAtWidth(tester, 1000);

    expect(find.text('screen:home'), findsOneWidget);
    expect(find.byType(NavigationDrawer), findsOneWidget);
    // Nothing is selected: the form is where you are, not somewhere you went.
    expect(
      tester.widget<NavigationDrawer>(find.byType(NavigationDrawer)).selectedIndex,
      isNull,
    );
  });

  testWidgets('entering an expense is not a destination', (tester) async {
    await pumpAtWidth(tester, 1000);

    expect(find.text('Expenses'), findsOneWidget);
    expect(find.text('Add'), findsNothing);
  });

  testWidgets('compact windows get a bottom bar once off the form',
      (tester) async {
    await pumpAtWidth(tester, 599);

    // The form has the screen to itself; there is no "nothing selected" state
    // for a NavigationBar to show.
    expect(find.byType(NavigationBar), findsNothing);

    await tester.tap(find.byTooltip('Close'));
    await tester.pumpAndSettle();

    expect(find.byType(NavigationBar), findsOneWidget);
    expect(find.text('screen:${Destination.expenses.label}'), findsOneWidget);
    expect(find.byType(NavigationRail), findsNothing);
    expect(find.byType(NavigationDrawer), findsNothing);
  });

  testWidgets('medium windows get a rail, and keep the form in the body',
      (tester) async {
    await pumpAtWidth(tester, 700);

    expect(find.byType(NavigationRail), findsOneWidget);
    expect(find.text('screen:home'), findsOneWidget);
    expect(find.byType(NavigationBar), findsNothing);
    expect(find.byType(NavigationDrawer), findsNothing);
  });

  testWidgets('the selection drives the screen', (tester) async {
    await pumpAtWidth(tester, 1000);

    await tester.tap(find.text(Destination.budget.label));
    await tester.pumpAndSettle();

    expect(find.text('screen:${Destination.budget.label}'), findsOneWidget);
    expect(find.text('screen:home'), findsNothing);
  });

  testWidgets('the primary action returns to the form', (tester) async {
    for (final width in <double>[599, 700, 1000]) {
      await pumpAtWidth(tester, width);
      if (width == 599) {
        await tester.tap(find.byTooltip('Close'));
      } else {
        await tester.tap(find.text(Destination.budget.label));
      }
      await tester.pumpAndSettle();
      expect(find.text('screen:home'), findsNothing, reason: 'width $width');

      // By type, not by tooltip: the extended button at expanded widths shows
      // its label, and a visible label is the name — a tooltip repeating it
      // would be noise to a screen reader.
      await tester.tap(find.byType(FloatingActionButton));
      await tester.pumpAndSettle();

      expect(find.text('screen:home'), findsOneWidget, reason: 'width $width');
    }
  });

  testWidgets('the same selection survives a resize across a breakpoint',
      (tester) async {
    await pumpAtWidth(tester, 599);
    await tester.tap(find.byTooltip('Close'));
    await tester.pumpAndSettle();
    await tester.tap(find.text(Destination.inbox.label));
    await tester.pumpAndSettle();

    tester.view.physicalSize = const Size(1000, 800);
    await tester.pumpAndSettle();

    expect(find.byType(NavigationDrawer), findsOneWidget);
    expect(find.text('screen:${Destination.inbox.label}'), findsOneWidget);
  });
}

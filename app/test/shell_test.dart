import 'package:app/src/destinations.dart';
import 'package:app/src/shell.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// The navigation component is a function of the window size class, so the
/// test sets a window size and asks which one appeared. The breakpoints are
/// Material 3's; these widths sit either side of them.
Future<List<String>> pumpAtWidth(WidgetTester tester, double width) async {
  final started = <String>[];
  tester.view.physicalSize = Size(width, 800);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    MaterialApp(
      home: AppShell(
        screenBuilder: (context, destination) =>
            Text('screen:${destination.label}'),
        onNewExpense: (context) => started.add('new'),
      ),
    ),
  );
  // Settle before asserting: Scaffold animates its floating button out when a
  // resize moves it into the rail, so mid-flight there are briefly two.
  await tester.pumpAndSettle();
  return started;
}

/// Move to a destination by tapping its label in whichever navigation
/// component is on screen.
Future<void> goTo(WidgetTester tester, Destination d) async {
  await tester.tap(find.text(d.label).last);
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('opens on Home', (tester) async {
    await pumpAtWidth(tester, 1000);

    expect(find.text('screen:${Destination.home.label}'), findsOneWidget);
  });

  testWidgets('entering an expense is not a destination', (tester) async {
    await pumpAtWidth(tester, 1000);

    // Twice: the app bar title and the drawer's own label.
    expect(find.text('Home'), findsWidgets);
    expect(find.text('Add'), findsNothing);
  });

  testWidgets('compact windows get a bottom bar', (tester) async {
    await pumpAtWidth(tester, 599);

    expect(find.byType(NavigationBar), findsOneWidget);
    expect(find.byType(NavigationRail), findsNothing);
    expect(find.byType(NavigationDrawer), findsNothing);
  });

  testWidgets('medium windows get a rail', (tester) async {
    await pumpAtWidth(tester, 700);

    expect(find.byType(NavigationRail), findsOneWidget);
    expect(find.byType(NavigationBar), findsNothing);
    expect(find.byType(NavigationDrawer), findsNothing);
  });

  testWidgets('expanded windows get a standard drawer', (tester) async {
    await pumpAtWidth(tester, 1000);

    expect(find.byType(NavigationDrawer), findsOneWidget);
    expect(find.byType(NavigationRail), findsNothing);
    expect(find.byType(NavigationBar), findsNothing);
  });

  testWidgets('the selection drives the screen', (tester) async {
    await pumpAtWidth(tester, 1000);

    await tester.tap(find.text(Destination.budget.label));
    await tester.pumpAndSettle();

    expect(find.text('screen:${Destination.budget.label}'), findsOneWidget);
    expect(find.text('screen:${Destination.home.label}'), findsNothing);
  });

  testWidgets('the primary action is reachable at every width', (tester) async {
    for (final width in <double>[599, 700, 1000]) {
      final started = await pumpAtWidth(tester, width);

      // By type, not by tooltip: the extended button at expanded widths shows
      // its label, and a visible label is the name — a tooltip repeating it
      // would be noise to a screen reader.
      await tester.tap(find.byType(FloatingActionButton));
      await tester.pumpAndSettle();

      expect(started, ['new'], reason: 'width $width');
    }
  });

  testWidgets('the same selection survives a resize across a breakpoint',
      (tester) async {
    await pumpAtWidth(tester, 599);
    await tester.tap(find.text(Destination.inbox.label));
    await tester.pumpAndSettle();

    tester.view.physicalSize = const Size(1000, 800);
    await tester.pumpAndSettle();

    expect(find.byType(NavigationDrawer), findsOneWidget);
    expect(find.text('screen:${Destination.inbox.label}'), findsOneWidget);
  });

  testWidgets('the drawer is headed with the app name', (tester) async {
    // The one place the app says its own name inside the app. Worth pinning:
    // a rename that misses here leaves the old name in the only spot a user
    // reads it every time they navigate.
    await pumpAtWidth(tester, 1400);

    expect(
      find.descendant(
        of: find.byType(NavigationDrawer),
        matching: find.text('Raven Budgeting'),
      ),
      findsOneWidget,
    );
  });

  testWidgets('the new-expense button is not on Budget or Settings',
      (tester) async {
    // Budget already has a primary action of its own — "Set a budget" — and
    // Material 3 allows a screen one. A second, heavier button offering
    // something unrelated is the one thing Budget was not opened to do.
    // Settings has no entry action at all, so the button there was only ever
    // something to hit by accident.
    for (final double width in [400.0, 700.0, 1400.0]) {
      await pumpAtWidth(tester, width);

      for (final d in [Destination.budget, Destination.settings]) {
        await goTo(tester, d);
        expect(
          find.byType(FloatingActionButton),
          findsNothing,
          reason: '${d.label} at ${width.toInt()}px',
        );
      }
    }
  });

  testWidgets('the new-expense button is on the screens about spending',
      (tester) async {
    // The overview, the list, and the queue of things not categorized yet:
    // the three places where the next thing you do is record what was spent.
    for (final double width in [400.0, 700.0, 1400.0]) {
      await pumpAtWidth(tester, width);

      for (final d in [
        Destination.home,
        Destination.expenses,
        Destination.inbox,
      ]) {
        await goTo(tester, d);
        expect(
          find.byType(FloatingActionButton),
          findsOneWidget,
          reason: '${d.label} at ${width.toInt()}px',
        );
      }
    }
  });

  testWidgets('the rail keeps its place when a screen offers no button',
      (tester) async {
    // The button lives in the rail's leading slot at medium widths, so a
    // screen without one leaves a hole there. The destinations must not slide
    // up into it: navigation that moves as you navigate is navigation you
    // have to re-find every time.
    // Scoped to the rail: the app bar carries the same word, and which of the
    // two moved is the whole question.
    final inRail = find.descendant(
      of: find.byType(NavigationRail),
      matching: find.text('Home'),
    );

    await pumpAtWidth(tester, 700);
    final double before = tester.getRect(inRail).top;

    await goTo(tester, Destination.budget);

    expect(tester.getRect(inRail).top, before);
  });

  testWidgets('the button floats in the lower right at expanded widths',
      (tester) async {
    // The same corner the compact window uses, rather than inside the
    // navigation. Material 3 suggests considering the upper left here and
    // then says to test whether it is really best; it allows a FAB aligned
    // left, center or right, and this app is aligned right everywhere.
    await pumpAtWidth(tester, 1400);

    expect(
      find.descendant(
        of: find.byType(NavigationDrawer),
        matching: find.byType(FloatingActionButton),
      ),
      findsNothing,
    );

    final Rect fab = tester.getRect(find.byType(FloatingActionButton));
    expect(fab.right, closeTo(1400 - 16, 0.5));
    expect(fab.bottom, closeTo(800 - 16, 0.5));
  });

  testWidgets('the floating button is the medium size', (tester) async {
    // 80dp, which is the medium FAB Material 3 asks for in a mobile layout
    // and the size Flutter has no constructor for. Checked here rather than
    // trusted to the theme override: the whole point of the override is that
    // nothing else in the framework will tell you it stopped working.
    await pumpAtWidth(tester, 400);

    expect(tester.getSize(find.byType(FloatingActionButton)), const Size(80, 80));
  });
}

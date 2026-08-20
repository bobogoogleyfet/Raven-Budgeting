import 'dart:math' as math;

import 'package:app/src/widgets/budget_ring.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

RingSegment segment(String label, int budgeted, int spent, {bool over = false}) =>
    RingSegment(
      label: label,
      budgeted: budgeted,
      spent: spent,
      isOver: over,
      colour: Colors.blue,
    );

Future<List<int?>> pumpRing(
  WidgetTester tester,
  List<RingSegment> segments, {
  double size = 300,
}) async {
  final taps = <int?>[];
  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: Center(
          child: SizedBox(
            width: size,
            height: size,
            child: BudgetRing(
              segments: segments,
              centre: const Text('centre'),
              onSelect: taps.add,
            ),
          ),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return taps;
}

/// A point on the ring, `degrees` clockwise from twelve o'clock.
///
/// Mid-segment rather than on a boundary: the gaps between segments are as
/// wide as the stroke, because a round cap overhangs its own arc and a
/// narrower gap lets neighbours touch.
Offset onRing(Rect ring, double degrees) {
  const double thickness = 20;
  final double radius = ring.shortestSide / 2 - thickness / 2;
  final double radians = (degrees - 90) * math.pi / 180;
  return ring.center +
      Offset(math.cos(radians) * radius, math.sin(radians) * radius);
}

void main() {
  testWidgets('a tap at twelve o clock lands on the first segment',
      (tester) async {
    // The ring starts at the top and runs clockwise; a ring that started at
    // three would read as having been rotated.
    final taps = await pumpRing(tester, [
      segment('Groceries', 100, 50),
      segment('Gas', 100, 50),
    ]);

    // Two equal budgets: the first owns twelve o'clock round to six.
    final Rect ring = tester.getRect(find.byType(BudgetRing));
    await tester.tapAt(onRing(ring, 45));
    await tester.pumpAndSettle();

    expect(taps, [0]);
  });

  testWidgets('a tap at six o clock lands on the second of two segments',
      (tester) async {
    final taps = await pumpRing(tester, [
      segment('Groceries', 100, 50),
      segment('Gas', 100, 50),
    ]);

    final Rect ring = tester.getRect(find.byType(BudgetRing));
    await tester.tapAt(onRing(ring, 225));
    await tester.pumpAndSettle();

    expect(taps, [1]);
  });

  testWidgets('the arcs are proportional to the budgets, not to their count',
      (tester) async {
    // Three quarters of the circle belongs to the first budget: clockwise from
    // the top, it runs past three o'clock and six o'clock and ends at nine.
    // So a point just *below* nine o'clock is still its, and a point just
    // above is already the second's.
    final taps = await pumpRing(tester, [
      segment('Rent', 300, 0),
      segment('Gas', 100, 0),
    ]);

    // Three quarters of the circle belongs to the first budget: clockwise from
    // the top it runs past three and six o'clock and ends at nine. So 135° is
    // still its, and 315° — up past nine — is the second's.
    final Rect ring = tester.getRect(find.byType(BudgetRing));
    await tester.tapAt(onRing(ring, 135));
    await tester.pumpAndSettle();

    expect(taps, [0]);

    await tester.tapAt(onRing(ring, 315));
    await tester.pumpAndSettle();
    expect(taps, [0, 1]);
  });

  testWidgets('a tap in the hole selects nothing', (tester) async {
    // The middle belongs to the figure, which is the point of the ring.
    final taps = await pumpRing(tester, [
      segment('Groceries', 100, 50),
      segment('Gas', 100, 50),
    ]);

    final Rect ring = tester.getRect(find.byType(BudgetRing));
    await tester.tapAt(ring.center);
    await tester.pumpAndSettle();

    expect(taps, [null]);
  });

  testWidgets('nothing budgeted paints no segments and still shows the middle',
      (tester) async {
    await pumpRing(tester, const []);

    expect(find.text('centre'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('a zero-budget segment does not divide by zero', (tester) async {
    await pumpRing(tester, [segment('Groceries', 0, 5000)]);

    expect(tester.takeException(), isNull);
  });
}

import 'package:app/src/widgets/budget_ring.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

RingSegment segment(String label, int budgeted, int spent, {bool over = false}) =>
    RingSegment(label: label, budgeted: budgeted, spent: spent, isOver: over);

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

void main() {
  testWidgets('a tap at twelve o clock lands on the first segment',
      (tester) async {
    // The ring starts at the top and runs clockwise; a ring that started at
    // three would read as having been rotated.
    final taps = await pumpRing(tester, [
      segment('Groceries', 100, 50),
      segment('Gas', 100, 50),
    ]);

    final Rect ring = tester.getRect(find.byType(BudgetRing));
    await tester.tapAt(Offset(ring.center.dx + 4, ring.top + 12));
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
    await tester.tapAt(Offset(ring.center.dx - 4, ring.bottom - 12));
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

    final Rect ring = tester.getRect(find.byType(BudgetRing));
    await tester.tapAt(Offset(ring.left + 12, ring.center.dy + 4));
    await tester.pumpAndSettle();

    expect(taps, [0]);

    await tester.tapAt(Offset(ring.left + 12, ring.center.dy - 4));
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

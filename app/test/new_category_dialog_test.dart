import 'package:app/src/api.dart';
import 'package:app/src/screens/new_category_dialog.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

class Made {
  Made(this.groupId, this.name, this.taxable);

  final int groupId;
  final String name;
  final bool taxable;
}

Future<List<Made>> pump(WidgetTester tester, {Object? error}) async {
  final made = <Made>[];
  final groups = <CategoryGroupDto>[
    CategoryGroupDto(id: 1, name: 'Bills', isHidden: false),
    CategoryGroupDto(id: 3, name: 'Occasional', isHidden: false),
    CategoryGroupDto(id: 5, name: 'Education', isHidden: false),
  ];
  tester.view.physicalSize = const Size(900, 1100);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);

  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: NewCategoryDialog(
          loadGroups: () async => groups,
          // Returned, not appended: the dialog is what adds it to the list it
          // is showing, and doing both would offer the group twice.
          createGroup: (name) async =>
              CategoryGroupDto(id: 9, name: name, isHidden: false),
          create: ({
            required int groupId,
            required String name,
            required bool taxable,
          }) async {
            if (error != null) throw error;
            made.add(Made(groupId, name, taxable));
            return CategoryDto(
              id: 99,
              name: name,
              groupName: groupId == 1 ? 'Bills' : 'Occasional',
              isBudgeted: true,
            );
          },
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return made;
}

Future<void> chooseGroup(WidgetTester tester, String name) async {
  await tester.tap(find.byType(DropdownButtonFormField<int>));
  await tester.pumpAndSettle();
  await tester.tap(find.text(name).last);
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('a bill is recorded as already carrying its tax', (tester) async {
    final made = await pump(tester);

    await tester.enterText(find.byType(TextField), 'Car Payment');
    await chooseGroup(tester, 'Bills');
    await tester.tap(find.text('Already in the amount'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'Add category'));
    await tester.pumpAndSettle();

    // A car payment taxed at 7.45% is $537.25 where the statement says
    // $500.00, and nothing on screen would say why.
    expect(made.single.name, 'Car Payment');
    expect(made.single.groupId, 1);
    expect(made.single.taxable, isFalse);
  });

  testWidgets('shopping keeps the till adding the tax', (tester) async {
    final made = await pump(tester);

    await tester.enterText(find.byType(TextField), 'Pet Food');
    await chooseGroup(tester, 'Bills');
    await tester.tap(find.widgetWithText(FilledButton, 'Add category'));
    await tester.pumpAndSettle();

    expect(made.single.taxable, isTrue);
  });

  testWidgets('a group says what it is for, not just when it happens',
      (tester) async {
    await pump(tester);
    await chooseGroup(tester, 'Occasional');

    // The old name described a schedule; the purpose is what someone is
    // choosing between.
    expect(find.textContaining('Money set aside each month'), findsOneWidget);
  });

  testWidgets('Education is offered, and says why it is empty', (tester) async {
    await pump(tester);
    await chooseGroup(tester, 'Education');

    expect(
      find.textContaining('a different set of costs for everyone'),
      findsOneWidget,
    );
  });

  testWidgets('a group can be made when none of them fit', (tester) async {
    final made = await pump(tester);

    await tester.tap(find.byTooltip('New group'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField).last, 'Sabbatical');
    await tester.tap(find.widgetWithText(FilledButton, 'Add group'));
    await tester.pumpAndSettle();

    // Made and selected: nobody makes a group and then wants to go looking
    // for it.
    await tester.enterText(find.byType(TextField).first, 'Course Fees');
    await tester.tap(find.widgetWithText(FilledButton, 'Add category'));
    await tester.pumpAndSettle();

    expect(made.single.groupId, 9);
    expect(made.single.name, 'Course Fees');
  });

  testWidgets('a nameless category is refused', (tester) async {
    final made = await pump(tester);
    await chooseGroup(tester, 'Bills');
    await tester.tap(find.widgetWithText(FilledButton, 'Add category'));
    await tester.pumpAndSettle();

    expect(made, isEmpty);
    expect(find.textContaining('Give the category a name'), findsOneWidget);
  });

  testWidgets('one with nowhere to belong is refused', (tester) async {
    final made = await pump(tester);
    await tester.enterText(find.byType(TextField), 'Car Payment');
    await tester.tap(find.widgetWithText(FilledButton, 'Add category'));
    await tester.pumpAndSettle();

    expect(made, isEmpty);
    expect(find.textContaining('Pick where it belongs'), findsOneWidget);
  });

  testWidgets('a refused create says why and stays open', (tester) async {
    final made = await pump(tester, error: Exception('invalid input: category name is empty'));
    await tester.enterText(find.byType(TextField), 'Car Payment');
    await chooseGroup(tester, 'Bills');
    await tester.tap(find.widgetWithText(FilledButton, 'Add category'));
    await tester.pumpAndSettle();

    expect(made, isEmpty);
    expect(find.textContaining('invalid input'), findsOneWidget);
  });
}

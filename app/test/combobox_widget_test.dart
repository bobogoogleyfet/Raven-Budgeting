import 'package:app/src/widgets/combobox.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

Future<List<String>> pump(
  WidgetTester tester, {
  bool strict = false,
  bool openOnFocus = true,
  String? noMatchLabel,
  List<String> options = const ['Highland', 'Salt Lake City', 'Provo'],
}) async {
  final committed = <String>[];
  final controller = TextEditingController();
  addTearDown(controller.dispose);

  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: Column(
          children: [
            Combobox(
              controller: controller,
              options: options,
              strict: strict,
              noMatchLabel: noMatchLabel,
              openOnFocus: openOnFocus,
              hint: 'Start typing…',
              emptyLabel: 'Start typing to find a city.',
              onCommit: committed.add,
            ),
            const TextField(key: Key('elsewhere')),
          ],
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return committed;
}

/// Leave the field, which is what commits it.
Future<void> leave(WidgetTester tester) async {
  await tester.tap(find.byKey(const Key('elsewhere')));
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('a search field shows nothing until something is typed',
      (tester) async {
    await pump(tester, strict: true, openOnFocus: false);

    await tester.tap(find.byType(TextField).first);
    await tester.pumpAndSettle();

    // Four hundred jurisdictions dropped over the form on focus helps nobody.
    expect(find.text('Salt Lake City'), findsNothing);

    await tester.enterText(find.byType(TextField).first, 'salt');
    await tester.pumpAndSettle();

    expect(find.text('Salt Lake City'), findsOneWidget);
  });

  testWidgets('a strict field refuses a name the list does not have',
      (tester) async {
    final committed = await pump(tester, strict: true, openOnFocus: false);

    await tester.enterText(find.byType(TextField).first, 'Highland, UT');
    await tester.pumpAndSettle();
    await leave(tester);

    // A name the state does not publish would be created at 0% and record no
    // tax on every receipt filed against it.
    expect(committed, isEmpty);
    expect(
      tester.widget<TextField>(find.byType(TextField).first).controller?.text,
      '',
      reason: 'the text it could not take is put back',
    );
  });

  testWidgets('a strict field keeps the last name it knew', (tester) async {
    final committed = await pump(tester, strict: true, openOnFocus: false);

    await tester.enterText(find.byType(TextField).first, 'Provo');
    await tester.pumpAndSettle();
    await leave(tester);
    expect(committed, ['Provo']);

    await tester.enterText(find.byType(TextField).first, 'Prov');
    await tester.pumpAndSettle();
    await leave(tester);

    expect(committed, ['Provo'], reason: 'half a name commits nothing');
    expect(
      tester.widget<TextField>(find.byType(TextField).first).controller?.text,
      'Provo',
    );
  });

  testWidgets('a strict field says so rather than offering to add',
      (tester) async {
    await pump(tester, strict: true, openOnFocus: false);

    await tester.enterText(find.byType(TextField).first, 'Boise');
    await tester.pumpAndSettle();

    // The widget's own fallback. Refusing without saying anything reads as a
    // dead field, so there is always a sentence.
    expect(find.textContaining('Nothing matches'), findsOneWidget);
    expect(find.textContaining('Press Enter to add'), findsNothing);
  });

  testWidgets('a strict field can say why in its own words', (tester) async {
    // The refusal differs per field — an unpublished city and a category that
    // does not exist are different problems with different fixes — so the
    // sentence is the caller's to supply rather than the widget's to guess.
    await pump(
      tester,
      strict: true,
      openOnFocus: false,
      noMatchLabel: 'No Utah jurisdiction matches “{typed}”.',
    );

    await tester.enterText(find.byType(TextField).first, 'Boise');
    await tester.pumpAndSettle();

    expect(find.text('No Utah jurisdiction matches “Boise”.'), findsOneWidget);
  });

  testWidgets('a store field still takes a name nobody has used before',
      (tester) async {
    final committed = await pump(
      tester,
      options: const ["Macey's", 'Costco'],
    );

    await tester.enterText(find.byType(TextField).first, 'Corner Shop');
    await tester.pumpAndSettle();
    await leave(tester);

    // The shop you are standing in is more authoritative than the list.
    expect(committed, ['Corner Shop']);
  });
}

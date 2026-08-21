import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:app/src/widgets/cents_field.dart';

void main() {
  for (final w in [110.0, 100.0, 92.0, 84.0, 76.0]) {
    testWidgets('cents field at $w', (tester) async {
      tester.view.physicalSize = const Size(400, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final c = TextEditingController(text: '154.32');
      await tester.pumpWidget(MaterialApp(
        home: Scaffold(
          body: Center(child: SizedBox(width: w, child: CentsField(controller: c, label: 'Amount'))),
        ),
      ));
      await tester.pumpAndSettle();
      final ed = tester.getSize(find.byType(EditableText));
      print('CENTS $w -> editable $ed  field ${tester.getSize(find.byType(TextField))}');
    });
  }
}

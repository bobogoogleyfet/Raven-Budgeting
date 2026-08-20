import 'package:app/src/widgets/cents_field.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

/// What the field would show after typing `keys` from empty.
String typing(String keys) {
  const formatter = CentsInputFormatter();
  var value = TextEditingValue.empty;
  for (final key in keys.split('')) {
    final typed = TextEditingValue(
      text: value.text + key,
      selection: TextSelection.collapsed(offset: value.text.length + 1),
    );
    value = formatter.formatEditUpdate(value, typed);
  }
  return value.text;
}

/// What it would show after deleting the last character.
String backspace(String from) {
  const formatter = CentsInputFormatter();
  final before = TextEditingValue(
    text: from,
    selection: TextSelection.collapsed(offset: from.length),
  );
  final after = TextEditingValue(
    text: from.substring(0, from.length - 1),
    selection: TextSelection.collapsed(offset: from.length - 1),
  );
  return formatter.formatEditUpdate(before, after).text;
}

void main() {
  test('digits fill from the cents up', () {
    expect(typing('5'), '0.05');
    expect(typing('50'), '0.50');
    expect(typing('150'), '1.50');
    expect(typing('728'), '7.28');
    expect(typing('14390'), '143.90');
  });

  test('it keeps grouping past a thousand', () {
    expect(typing('123456'), '1,234.56');
    expect(typing('100000'), '1,000.00');
  });

  test('a decimal point typed out of habit changes nothing', () {
    // The point is where the digits have got to, not a key.
    expect(typing('7.28'), '7.28');
    expect(typing('250.00'), '250.00');
  });

  test('deleting shifts the digits back down', () {
    expect(backspace('1.50'), '0.15');
    expect(backspace('0.15'), '0.01');
    expect(backspace('0.01'), '');
  });

  test('an empty field is empty, not zero', () {
    // So the hint can say what the format is, and so a row still being typed
    // is left out of the draft rather than sent as $0.00.
    expect(typing(''), '');
  });

  test('a refund keeps its minus', () {
    expect(typing('-500'), '-5.00');
    expect(typing('-'), '-');
  });

  test('junk contributes nothing', () {
    expect(typing('abc'), '');
    expect(typing('1a2'), '0.12');
  });
}

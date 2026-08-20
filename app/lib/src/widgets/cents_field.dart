import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../money.dart';

/// Fills an amount from the right, the way a till does.
///
/// Typing `5` is five cents, `50` is fifty, `150` is a dollar fifty. Nobody
/// entering a receipt types a decimal point — the point is where the digits
/// have got to, not a key — and a field that waits for one makes `1` mean a
/// dollar until proven otherwise.
///
/// The text is always a formatted amount, so what is on screen is what
/// [parseCents] will read back. Deleting is what it looks like: the digits
/// shift right again, and an empty field is empty rather than `0.00`, so the
/// hint can say what the format is.
class CentsInputFormatter extends TextInputFormatter {
  const CentsInputFormatter();

  /// Enough for any receipt, and short of anything that would overflow.
  static const int _maxDigits = 12;

  @override
  TextEditingValue formatEditUpdate(
    TextEditingValue oldValue,
    TextEditingValue newValue,
  ) {
    // A refund is a negative amount, and the core takes them — so a leading
    // minus survives, wherever in the text it was typed.
    final bool negative = newValue.text.contains('-');
    // Leading zeros are dropped, so deleting the last digit empties the field
    // instead of sticking at `0.00`. That matters beyond tidiness: `0.00`
    // parses to zero, and a zero row would claim the user meant to file
    // nothing under that category, where an empty one is simply left out.
    final String digits = newValue.text
        .replaceAll(RegExp(r'[^0-9]'), '')
        .replaceFirst(RegExp(r'^0+'), '');

    if (digits.isEmpty) {
      return TextEditingValue(
        text: negative ? '-' : '',
        selection: TextSelection.collapsed(offset: negative ? 1 : 0),
      );
    }

    final String capped = digits.length > _maxDigits
        ? digits.substring(digits.length - _maxDigits)
        : digits;
    final int cents = int.parse(capped);
    final String text = formatCents(fromCore(negative ? -cents : cents));

    return TextEditingValue(
      text: text,
      // The caret stays at the end: the next digit always lands in the cents
      // and pushes everything left, so there is nowhere else for it to be.
      selection: TextSelection.collapsed(offset: text.length),
    );
  }
}

/// An amount field: right-aligned, tabular, and filled from the cents up.
class CentsField extends StatelessWidget {
  const CentsField({
    super.key,
    required this.controller,
    this.onChanged,
    this.onSubmitted,
    this.label,
    this.autofocus = false,
  });

  final TextEditingController controller;
  final ValueChanged<String>? onChanged;
  final ValueChanged<String>? onSubmitted;
  final String? label;
  final bool autofocus;

  @override
  Widget build(BuildContext context) {
    return TextField(
      controller: controller,
      autofocus: autofocus,
      onChanged: onChanged,
      onSubmitted: onSubmitted,
      textAlign: TextAlign.right,
      keyboardType: const TextInputType.numberWithOptions(decimal: true),
      inputFormatters: const [CentsInputFormatter()],
      decoration: InputDecoration(
        labelText: label,
        // What the field will hold, not an example of what to put in it: a
        // hint reading "250.00" is a number the reader has to decide not to
        // take literally.
        hintText: '0.00',
        prefixText: r'$',
        isDense: true,
        border: const OutlineInputBorder(),
      ),
    );
  }
}

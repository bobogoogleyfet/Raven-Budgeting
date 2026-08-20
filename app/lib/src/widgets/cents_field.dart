import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../api.dart' as api;
import '../money.dart';

/// How the parts of an amount are added up.
///
/// The core's, always, in the app. It exists as a variable only because
/// `flutter test` runs without the compiled library, and a field that cannot
/// be typed into in a widget test is a field whose behaviour goes untested —
/// which would be a worse trade than this one. There is still one
/// implementation of the arithmetic, and it is in Rust.
Cents Function(List<Cents> parts) addUp = api.totalOf;

/// The parts of an amount typed as `1.38+1.84`.
///
/// Parsing, not arithmetic: this splits what was typed and reads each piece.
/// What they come to is [totalOf], which is the core's answer.
///
/// A trailing `+` is a part not typed yet, not a zero — the field is mid-entry
/// and has nothing to report for it.
List<Cents> amountParts(String text) {
  final parts = <Cents>[];
  for (final piece in text.split('+')) {
    final cents = parseCents(piece);
    if (cents != null) parts.add(cents);
  }
  return parts;
}

/// What the field's text comes to, or null while it holds nothing usable.
Cents? amountValue(String text) {
  final parts = amountParts(text);
  if (parts.isEmpty) return null;
  // One figure is itself; several are the core's sum of them.
  if (parts.length == 1) return parts.single;
  return addUp(parts);
}

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
    // Several figures under one category — three items off one receipt — are
    // typed as they are read and added by the core. Each piece fills from the
    // cents up on its own.
    if (newValue.text.contains('+')) {
      final pieces = newValue.text.split('+');
      final formatted = [
        for (final piece in pieces) _oneAmount(piece),
      ].join(' + ');
      return TextEditingValue(
        text: formatted,
        selection: TextSelection.collapsed(offset: formatted.length),
      );
    }
    return _single(newValue);
  }

  /// One figure, formatted, or empty.
  String _oneAmount(String text) {
    final digits =
        text.replaceAll(RegExp(r'[^0-9]'), '').replaceFirst(RegExp(r'^0+'), '');
    if (digits.isEmpty) return text.contains('-') ? '-' : '';
    final capped = digits.length > _maxDigits
        ? digits.substring(digits.length - _maxDigits)
        : digits;
    final cents = int.parse(capped);
    return formatCents(fromCore(text.contains('-') ? -cents : cents));
  }

  TextEditingValue _single(TextEditingValue newValue) {
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

/// An amount field: right-aligned, filled from the cents up, and able to hold
/// several figures at once.
class CentsField extends StatefulWidget {
  const CentsField({
    super.key,
    required this.controller,
    this.onChanged,
    this.onSubmitted,
    this.label,
    this.autofocus = false,
    this.focusNode,
  });

  final TextEditingController controller;
  final ValueChanged<String>? onChanged;
  final ValueChanged<String>? onSubmitted;
  final String? label;
  final bool autofocus;
  final FocusNode? focusNode;

  @override
  State<CentsField> createState() => _CentsFieldState();
}

class _CentsFieldState extends State<CentsField> {
  @override
  void initState() {
    super.initState();
    widget.controller.addListener(_onText);
  }

  @override
  void dispose() {
    widget.controller.removeListener(_onText);
    super.dispose();
  }

  void _onText() => setState(() {});

  /// Shown only once there is more than one figure: a field holding `1.38`
  /// does not need to be told it comes to $1.38.
  String? get _runningTotal {
    final parts = amountParts(widget.controller.text);
    if (parts.length < 2) return null;
    return '= ${formatMoney(addUp(parts))}';
  }

  /// A key for the plus, because a phone's number pad may not have one.
  void _addPart() {
    final text = widget.controller.text;
    if (text.trim().isEmpty || text.trimRight().endsWith('+')) return;
    widget.controller.text = '$text + ';
    widget.controller.selection = TextSelection.collapsed(
      offset: widget.controller.text.length,
    );
    widget.onChanged?.call(widget.controller.text);
  }

  @override
  Widget build(BuildContext context) {
    return TextField(
      controller: widget.controller,
      focusNode: widget.focusNode,
      autofocus: widget.autofocus,
      onChanged: widget.onChanged,
      onSubmitted: widget.onSubmitted,
      textAlign: TextAlign.right,
      keyboardType: const TextInputType.numberWithOptions(decimal: true),
      inputFormatters: const [CentsInputFormatter()],
      decoration: InputDecoration(
        labelText: widget.label,
        // What the field will hold, not an example of what to put in it: a
        // hint reading "250.00" is a number the reader has to decide not to
        // take literally.
        hintText: '0.00',
        prefixText: r'$',
        // The sum, while there is one to show. It is the core's answer to what
        // was typed, not this field's opinion of it.
        helperText: _runningTotal,
        suffixIcon: IconButton(
          onPressed: _addPart,
          icon: const Icon(Icons.add, size: 18),
          tooltip: 'Add another amount',
        ),
        isDense: true,
        border: const OutlineInputBorder(),
      ),
    );
  }
}

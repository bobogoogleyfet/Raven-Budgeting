import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../dates.dart';

/// A date typed as three segments: MM / DD / YYYY.
///
/// Three real fields rather than one with hand-managed carets. Selecting a
/// segment whole so typing overwrites it, and putting the caret after the
/// digit just typed so backspace removes one character rather than four, is
/// exactly what a short native input already does — so this steers focus and
/// lets the platform handle the text.
///
/// A segment hands over as soon as it cannot take another digit: two digits,
/// or one that rules a second out — month 2 can only be February, day 4 can
/// only be the 4th. That is what makes `08202026` on a numpad a date rather
/// than a sequence of fields to tab between.
///
/// The value is `YYYY-MM-DD`, or empty while the date is incomplete or
/// impossible. [toIso] rejects the 30th of February rather than clamping it:
/// silently filing a receipt on a day nobody chose is worse than refusing.
class DateField extends StatefulWidget {
  const DateField({
    super.key,
    required this.value,
    required this.onChanged,
    this.label,
    this.autofocus = false,
    this.onSubmitted,
    this.focusNode,
  });

  /// `YYYY-MM-DD`, or empty.
  final String value;

  /// Fired with `YYYY-MM-DD` when the three segments make a real date, and
  /// with an empty string while they do not.
  final ValueChanged<String> onChanged;

  final String? label;

  /// Whether the month segment takes focus, selected, when the field appears.
  final bool autofocus;

  /// Enter moves on: the field is done, the form is not.
  final VoidCallback? onSubmitted;

  /// Drives the *month* segment, which is where the field starts.
  ///
  /// Exists so a form can put the cursor back here after a save without
  /// rebuilding the widget — `autofocus` fires once, when the field first
  /// appears, and a cleared form is the same field it always was. Focusing
  /// the segment selects it whole (see [initState]), so the date comes back
  /// highlighted and the next receipt can be typed straight over it.
  ///
  /// The caller owns it and must dispose it; when null the field makes and
  /// disposes its own.
  final FocusNode? focusNode;

  @override
  State<DateField> createState() => _DateFieldState();
}

class _DateFieldState extends State<DateField> {
  final TextEditingController _month = TextEditingController();
  final TextEditingController _day = TextEditingController();
  final TextEditingController _year = TextEditingController();
  late final FocusNode _monthFocus = widget.focusNode ?? FocusNode();
  final FocusNode _dayFocus = FocusNode();
  final FocusNode _yearFocus = FocusNode();

  @override
  void initState() {
    super.initState();
    _adopt(widget.value);
    for (final (node, controller) in [
      (_monthFocus, _month),
      (_dayFocus, _day),
      (_yearFocus, _year),
    ]) {
      // Focusing a segment selects it whole, so typing replaces it.
      node.addListener(() {
        if (node.hasFocus) {
          controller.selection = TextSelection(
            baseOffset: 0,
            extentOffset: controller.text.length,
          );
        }
      });
    }
  }

  @override
  void didUpdateWidget(DateField old) {
    super.didUpdateWidget(old);
    // Only when the value came from outside — the form resetting after a save.
    // Adopting our own emissions would fight the caret.
    if (widget.value != old.value && widget.value != _current()) {
      _adopt(widget.value);
    }
  }

  @override
  void dispose() {
    for (final c in [_month, _day, _year]) {
      c.dispose();
    }
    for (final n in [_dayFocus, _yearFocus]) {
      n.dispose();
    }
    // Only the one we made. Disposing a node the caller owns would leave them
    // holding a dead object, and they may well outlive this field.
    if (widget.focusNode == null) {
      _monthFocus.dispose();
    }
    super.dispose();
  }

  void _adopt(String iso) {
    final parts = fromIso(iso);
    _month.text = parts.month;
    _day.text = parts.day;
    _year.text = parts.year;
  }

  String _current() => toIso(_month.text, _day.text, _year.text) ?? '';

  void _emit() => widget.onChanged(_current());

  /// Whether this segment can still take another digit.
  ///
  /// A month starting with anything past 1 has no second digit to give, and
  /// neither has a day starting past 3. Waiting for one costs a keystroke on
  /// every date typed in a hurry.
  bool _full(String text, {required int width, required int firstDigitCap}) {
    if (text.length >= width) return true;
    if (text.length == 1) {
      final int first = int.tryParse(text) ?? 0;
      return first > firstDigitCap;
    }
    return false;
  }

  void _onSegment(
    String text, {
    required int width,
    required int firstDigitCap,
    FocusNode? next,
  }) {
    _emit();
    if (next != null && _full(text, width: width, firstDigitCap: firstDigitCap)) {
      next.requestFocus();
    }
  }

  /// Backspace in an empty segment steps back, so a mistyped day is one key
  /// away from being retyped rather than a reach for the mouse. Enter moves
  /// on: to the next segment, or out of the field when there is none.
  KeyEventResult _onKey(
    FocusNode? previous,
    FocusNode? next,
    TextEditingController own,
    KeyEvent event,
  ) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;

    if (event.logicalKey == LogicalKeyboardKey.enter ||
        event.logicalKey == LogicalKeyboardKey.numpadEnter) {
      if (next != null) {
        next.requestFocus();
      } else {
        widget.onSubmitted?.call();
      }
      return KeyEventResult.handled;
    }

    if (event.logicalKey != LogicalKeyboardKey.backspace) {
      return KeyEventResult.ignored;
    }
    if (own.text.isNotEmpty || previous == null) return KeyEventResult.ignored;
    previous.requestFocus();
    return KeyEventResult.handled;
  }

  Future<void> _pick() async {
    final DateTime initial =
        DateTime.tryParse(_current().isEmpty ? today() : _current()) ??
            DateTime.now();
    final picked = await showDatePicker(
      context: context,
      initialDate: initial,
      firstDate: DateTime(2000),
      lastDate: DateTime(2100),
    );
    if (picked == null) return;
    setState(() => _adopt(today(picked)));
    _emit();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Row(
      children: [
        _Segment(
          controller: _month,
          focus: _monthFocus,
          width: 46,
          maxLength: 2,
          hint: 'MM',
          autofocus: widget.autofocus,
          onKey: (node, event) => _onKey(null, _dayFocus, _month, event),
          onChanged: (text) => _onSegment(
            text,
            width: 2,
            firstDigitCap: 1,
            next: _dayFocus,
          ),
        ),
        Text(' / ', style: theme.textTheme.bodyLarge),
        _Segment(
          controller: _day,
          focus: _dayFocus,
          width: 46,
          maxLength: 2,
          hint: 'DD',
          onKey: (node, event) => _onKey(_monthFocus, _yearFocus, _day, event),
          onChanged: (text) => _onSegment(
            text,
            width: 2,
            firstDigitCap: 3,
            next: _yearFocus,
          ),
        ),
        Text(' / ', style: theme.textTheme.bodyLarge),
        _Segment(
          controller: _year,
          focus: _yearFocus,
          width: 68,
          maxLength: 4,
          hint: 'YYYY',
          onKey: (node, event) => _onKey(_dayFocus, null, _year, event),
          onChanged: (text) => _onSegment(text, width: 4, firstDigitCap: 9),
        ),
        IconButton(
          onPressed: _pick,
          icon: const Icon(Icons.calendar_today_outlined),
          tooltip: 'Pick a date',
        ),
      ],
    );
  }
}

class _Segment extends StatelessWidget {
  const _Segment({
    required this.controller,
    required this.focus,
    required this.width,
    required this.maxLength,
    required this.hint,
    required this.onChanged,
    required this.onKey,
    this.autofocus = false,
  });

  final TextEditingController controller;
  final FocusNode focus;
  final double width;
  final int maxLength;
  final String hint;
  final ValueChanged<String> onChanged;
  final KeyEventResult Function(FocusNode node, KeyEvent event) onKey;
  final bool autofocus;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: width,
      child: Focus(
        onKeyEvent: onKey,
        child: TextField(
          controller: controller,
          focusNode: focus,
          autofocus: autofocus,
          onChanged: onChanged,
          textAlign: TextAlign.center,
          keyboardType: const TextInputType.numberWithOptions(signed: false),
          inputFormatters: [
            FilteringTextInputFormatter.digitsOnly,
            LengthLimitingTextInputFormatter(maxLength),
          ],
          decoration: InputDecoration(
            hintText: hint,
            isDense: true,
            counterText: '',
            border: const OutlineInputBorder(),
            contentPadding:
                const EdgeInsets.symmetric(horizontal: 6, vertical: 12),
          ),
        ),
      ),
    );
  }
}

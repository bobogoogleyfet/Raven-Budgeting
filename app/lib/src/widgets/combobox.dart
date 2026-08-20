import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

/// Options that match `query`, best first.
///
/// Prefix matches come before interior ones, so typing "ma" offers Macey's
/// before Walmart — the store whose *name* starts that way is nearly always
/// the one meant, and burying it under an alphabetical accident costs a scroll
/// on every entry. Ties keep the input order, which is most-recently-shopped
/// first.
List<String> filterOptions(List<String> all, String query) {
  final String needle = query.trim().toLowerCase();
  if (needle.isEmpty) return List<String>.of(all);

  final prefix = <String>[];
  final interior = <String>[];
  for (final option in all) {
    final String candidate = option.toLowerCase();
    if (candidate.startsWith(needle)) {
      prefix.add(option);
    } else if (candidate.contains(needle)) {
      interior.add(option);
    }
  }
  return [...prefix, ...interior];
}

/// Whether `value` is already one of the options.
///
/// Exact, case-insensitively: a prefix is not a match, because "Cost Plus" is
/// a different shop from Costco and the half-typed name must still offer to be
/// added.
bool isKnown(List<String> all, String value) {
  final String needle = value.trim().toLowerCase();
  if (needle.isEmpty) return false;
  return all.any((option) => option.trim().toLowerCase() == needle);
}

/// A text field with a filtered dropdown.
///
/// Typing filters; the list is still browsable when the box is empty. A name
/// that matches nothing is kept as typed rather than rejected, because the
/// store you are standing in is more authoritative than the list.
class Combobox extends StatefulWidget {
  const Combobox({
    super.key,
    required this.controller,
    required this.options,
    required this.onCommit,
    this.label,
    this.hint,
    this.emptyLabel = 'Nothing here yet.',
    this.noMatchLabel,
    this.onSubmitted,
    this.openOnFocus = true,
    this.strict = false,
    this.focusNode,
  });

  final TextEditingController controller;
  final List<String> options;

  /// Fired when the value settles: a pick, or Enter, or leaving the field.
  final void Function(String value) onCommit;

  final String? label;
  final String? hint;

  /// What the list says when the box is empty and there is nothing to offer.
  final String emptyLabel;

  /// What a [strict] list says when what was typed matches nothing.
  ///
  /// Needed per field, because the reason a name is refused differs and a
  /// refusal that cannot say why reads as a bug: an unpublished city and a
  /// category that has not been created yet are different problems with
  /// different fixes. `{typed}` is replaced with what the user typed.
  final String? noMatchLabel;

  /// Enter on a committed value, for moving on through the form.
  final VoidCallback? onSubmitted;

  /// Whether the list appears as soon as the field is focused.
  ///
  /// False for a list of four hundred jurisdictions, where opening on focus
  /// drops a wall of names nobody asked for over the form. Typing is the
  /// search, the way a map's search box works.
  final bool openOnFocus;

  /// Whether only a name from the list can be committed.
  ///
  /// A store's name is the user's to invent — the shop they are standing in is
  /// more authoritative than the list. A *place* is not: the tax rate comes
  /// from a published jurisdiction, and a name that matches none of them would
  /// be created at 0% and quietly record no tax on every receipt filed against
  /// it. Nor is a *category*: the chart of accounts is the structure every
  /// report is grouped by, and a typo entered against a receipt would become a
  /// permanent one-row heading nobody meant to create.
  ///
  /// So a strict field keeps the last name it knew rather than taking one it
  /// does not, and never offers to create anything.
  final bool strict;

  /// Supplied when something else needs to hand focus to this field — the form
  /// walking from one field to the next as Enter is pressed.
  final FocusNode? focusNode;

  @override
  State<Combobox> createState() => _ComboboxState();
}

class _ComboboxState extends State<Combobox> {
  late final FocusNode _focus = widget.focusNode ?? FocusNode();
  late final bool _ownsFocus = widget.focusNode == null;
  final LayerLink _link = LayerLink();
  OverlayEntry? _overlay;

  /// Options added by typing a name nobody has used before. They join the list
  /// immediately, so the name is offered for the rest of the session, and only
  /// reach the database on save — a store row for a receipt the user abandoned
  /// would be debris in the picker forever.
  final List<String> _added = [];

  /// Which row the arrow keys are on. -1 means "none", so Enter takes what was
  /// typed rather than whatever happened to be at the top.
  int _active = -1;

  /// The last value this field actually committed, for a strict field to fall
  /// back to when the text is left as something it cannot accept.
  String _settled = '';

  List<String> get _all => [..._added, ...widget.options];

  List<String> get _shown => filterOptions(_all, widget.controller.text);

  @override
  void initState() {
    super.initState();
    _focus.addListener(_onFocusChange);
    widget.controller.addListener(_onTextChange);
  }

  @override
  void dispose() {
    _close();
    _focus.removeListener(_onFocusChange);
    widget.controller.removeListener(_onTextChange);
    if (_ownsFocus) _focus.dispose();
    super.dispose();
  }

  void _onFocusChange() {
    if (_focus.hasFocus) {
      if (widget.openOnFocus) _open();
      return;
    }

    _close();
    final String typed = widget.controller.text.trim();
    if (widget.strict && typed.isNotEmpty && !isKnown(_all, typed)) {
      // Half a name, or a name the published list does not have. Putting back
      // the last one it knew is the only honest answer: the alternative is a
      // location invented at 0%, which records no tax and says nothing.
      widget.controller.text = _settled;
      return;
    }
    // Leaving the field settles the value, whether or not a letter changed.
    _settled = typed;
    widget.onCommit(typed);
  }

  void _onTextChange() {
    _active = -1;
    if (_focus.hasFocus) {
      // A strict field is a search box: nothing to show until there is
      // something to search for.
      if (widget.openOnFocus || widget.controller.text.trim().isNotEmpty) {
        _open();
      } else {
        _close();
      }
    }
    _overlay?.markNeedsBuild();
  }

  void _open() {
    if (_overlay != null) return;
    _overlay = OverlayEntry(builder: _buildList);
    Overlay.of(context).insert(_overlay!);
  }

  void _close() {
    _overlay?.remove();
    _overlay = null;
  }

  void _move(int delta) {
    final shown = _shown;
    if (shown.isEmpty) return;
    _open();
    setState(() {
      _active += delta;
      // Wrapping through -1, so arrowing past the end returns to what was
      // typed rather than to the top of the list.
      if (_active < -1) {
        _active = shown.length - 1;
      } else if (_active >= shown.length) {
        _active = -1;
      }
    });
    _overlay?.markNeedsBuild();
  }

  void _commit(String value) {
    _settled = value;
    widget.controller.text = value;
    widget.controller.selection =
        TextSelection.collapsed(offset: value.length);
    _close();
    widget.onCommit(value);
  }

  void _submit() {
    final shown = _shown;
    final String value = _active >= 0 && _active < shown.length
        ? shown[_active]
        : widget.controller.text.trim();
    if (value.isEmpty) {
      _close();
      return;
    }
    if (widget.strict && !isKnown(_all, value)) {
      // Nothing to take. The list is the authority here.
      return;
    }
    if (!isKnown(_all, value)) _added.insert(0, value);
    _commit(value);
    widget.onSubmitted?.call();
  }

  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowDown:
        _move(1);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.arrowUp:
        _move(-1);
        return KeyEventResult.handled;
      case LogicalKeyboardKey.escape:
        // The value is untouched: Escape closes the list, it does not undo.
        _close();
        return KeyEventResult.handled;
      case LogicalKeyboardKey.enter:
      case LogicalKeyboardKey.numpadEnter:
        _submit();
        return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  Widget _buildList(BuildContext context) {
    final theme = Theme.of(context);
    final shown = _shown;
    final String typed = widget.controller.text.trim();

    return Positioned(
      width: (context.findRenderObject() as RenderBox?)?.size.width ?? 280,
      child: CompositedTransformFollower(
        link: _link,
        targetAnchor: Alignment.bottomLeft,
        followerAnchor: Alignment.topLeft,
        offset: const Offset(0, 4),
        child: Material(
          elevation: 3,
          borderRadius: BorderRadius.circular(8),
          clipBehavior: Clip.antiAlias,
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxHeight: 280),
            child: shown.isEmpty
                ? Padding(
                    padding: const EdgeInsets.all(12),
                    child: Text(
                      typed.isEmpty
                          ? widget.emptyLabel
                          : widget.strict
                              ? (widget.noMatchLabel ?? 'Nothing matches “{typed}”.')
                                  .replaceAll('{typed}', typed)
                              : 'Press Enter to add “$typed”',
                      style: theme.textTheme.bodyMedium?.copyWith(
                        color: theme.colorScheme.onSurfaceVariant,
                      ),
                    ),
                  )
                : ListView.builder(
                    primary: false,
                    shrinkWrap: true,
                    itemCount: shown.length,
                    itemBuilder: (context, index) => InkWell(
                      // A tap, not a press-and-release on the option itself:
                      // the field's own focus loss would close the list before
                      // a plain tap ever landed.
                      onTapDown: (_) => _commit(shown[index]),
                      child: Container(
                        color: index == _active
                            ? theme.colorScheme.primaryContainer
                                .withValues(alpha: 0.5)
                            : null,
                        padding: const EdgeInsets.symmetric(
                          horizontal: 12,
                          vertical: 10,
                        ),
                        child: Text(shown[index]),
                      ),
                    ),
                  ),
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return CompositedTransformTarget(
      link: _link,
      child: Focus(
        onKeyEvent: _onKey,
        child: TextField(
          controller: widget.controller,
          focusNode: _focus,
          decoration: InputDecoration(
            labelText: widget.label,
            hintText: widget.hint,
            border: const OutlineInputBorder(),
            isDense: true,
            suffixIcon: IconButton(
              // The same triangle the expense list sorts with: a dropdown
              // arrow and a sort arrow are the same idea, and two glyphs would
              // be two shapes.
              icon: const Icon(Icons.arrow_drop_down),
              tooltip: 'Show all',
              onPressed: () {
                if (_overlay == null) {
                  _focus.requestFocus();
                  _open();
                } else {
                  _close();
                }
              },
            ),
          ),
          onSubmitted: (_) => _submit(),
        ),
      ),
    );
  }
}

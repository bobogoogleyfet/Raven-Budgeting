import 'package:flutter/material.dart';

/// One value a column actually holds.
class FilterOption {
  const FilterOption({required this.value, required this.label});

  /// The id sent back to the core, as a string.
  final String value;

  final String label;
}

/// What to send to the core: `null` when nothing is hidden.
///
/// A filter listing every value and no filter at all return the same rows
/// *today*, and different rows tomorrow: an expense at a shop that did not
/// exist when the menu was built would fail the explicit list. Untouched has
/// to mean untouched.
///
/// The result keeps the source list's order — the menu is built alphabetically
/// and the query should not reshuffle it.
List<String>? narrowedTo(List<String> all, Set<String> hidden) {
  if (hidden.isEmpty) return null;
  return all.where((value) => !hidden.contains(value)).toList();
}

/// Drop hidden values that no longer exist.
///
/// A store that was removed cannot keep filtering the list from beyond the
/// grave — and while it lingered, the funnel would stay lit and [narrowedTo]
/// would keep sending an explicit list for a filter the user cannot see or
/// undo.
///
/// Pruning never triggers a reload by itself: it is a consequence of the data
/// changing, not of the user asking for anything.
Set<String> pruned(Set<String> hidden, List<FilterOption> options) {
  final live = options.map((o) => o.value).toSet();
  return hidden.where(live.contains).toSet();
}

/// A column filter, the way a spreadsheet does it: a funnel that opens a list
/// of tick boxes, everything ticked to begin with.
///
/// # Why ticking rather than choosing
///
/// A single-select filter answers "show me Costco". A column of tick boxes
/// answers "show me everything except Amazon", which is the question a list of
/// expenses is usually interrogated with, and it needs no separate "all" state
/// to be understood: nothing hidden is nothing ticked off.
///
/// The state is therefore a set of *hidden* values, not of selected ones.
class FilterMenuButton extends StatefulWidget {
  const FilterMenuButton({
    super.key,
    required this.label,
    required this.options,
    required this.hidden,
    required this.onChanged,
    this.dense = false,
  });

  final String label;
  final List<FilterOption> options;

  /// The values ticked off. Owned by the screen, because the query is built
  /// from every filter at once.
  final Set<String> hidden;

  final void Function(Set<String> hidden) onChanged;

  /// Whether to draw as a compact chip rather than a full-width header
  /// control.
  final bool dense;

  @override
  State<FilterMenuButton> createState() => _FilterMenuButtonState();
}

class _FilterMenuButtonState extends State<FilterMenuButton> {
  final MenuController _menu = MenuController();

  void _toggle(String value, bool ticked) {
    final next = Set<String>.from(widget.hidden);
    if (ticked) {
      next.remove(value);
    } else {
      next.add(value);
    }
    widget.onChanged(next);
  }

  void _showAll() => widget.onChanged(<String>{});

  void _hideAll() =>
      widget.onChanged(widget.options.map((o) => o.value).toSet());

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    // The funnel fills in when the column is filtered, so it is visible at a
    // glance which columns are hiding rows — the failure a filter invites is
    // forgetting it is on.
    final bool active = widget.hidden.isNotEmpty;
    final Color colour =
        active ? theme.colorScheme.primary : theme.colorScheme.onSurfaceVariant;

    return MenuAnchor(
      controller: _menu,
      menuChildren: [
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            TextButton(onPressed: _showAll, child: const Text('Show all')),
            TextButton(onPressed: _hideAll, child: const Text('Hide all')),
          ],
        ),
        const Divider(height: 1),
        ConstrainedBox(
          constraints: const BoxConstraints(maxHeight: 320, minWidth: 200),
          child: SingleChildScrollView(
            // Not the primary controller: the list behind this menu is already
            // using it, and two scroll positions on one controller is an
            // assertion on Android rather than a layout quirk.
            primary: false,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                for (final option in widget.options)
                  // A checkbox item rather than a menu item that closes: every
                  // tick reloads the list, and a menu that shut after each one
                  // would make "everything except these three" three trips.
                  CheckboxMenuButton(
                    value: !widget.hidden.contains(option.value),
                    onChanged: (ticked) => _toggle(option.value, ticked ?? true),
                    child: Text(option.label),
                  ),
              ],
            ),
          ),
        ),
      ],
      builder: (context, controller, child) {
        // The title and the funnel are one control. An 11-pixel icon is a
        // target you have to aim at; the whole thing being the button means
        // the name and the funnel do the same thing, which is also what they
        // look like they should do.
        final Widget content = Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Flexible(
              child: Text(
                widget.label,
                style: theme.textTheme.labelLarge?.copyWith(color: colour),
                overflow: TextOverflow.ellipsis,
              ),
            ),
            const SizedBox(width: 4),
            Icon(
              active ? Icons.filter_alt : Icons.filter_alt_outlined,
              size: 16,
              color: colour,
            ),
          ],
        );
        return TextButton(
          onPressed: () =>
              controller.isOpen ? controller.close() : controller.open(),
          style: TextButton.styleFrom(
            minimumSize: widget.dense ? null : const Size.fromHeight(40),
            padding: const EdgeInsets.symmetric(horizontal: 12),
            // Left, so the control sits over its own column rather than
            // floating in the middle of the space the column happens to have.
            alignment: widget.dense ? null : Alignment.centerLeft,
          ),
          child: content,
        );
      },
    );
  }
}

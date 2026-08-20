import 'package:flutter/material.dart';

import 'breakpoints.dart';
import 'destinations.dart';

/// The navigation shell.
///
/// Which navigation component shows is a function of the Material 3 window
/// size class: a bottom bar when the window is compact, a rail once there is
/// room beside the content, a standard drawer when there is room for labels
/// next to icons. All three drive the same selected destination, so the screen
/// underneath never knows which one is showing.
///
/// Entering an expense is the primary action rather than a destination, so it
/// is a button over the content — the rail's leading slot where there is one,
/// and a floating button where there is not. It is reachable from every
/// screen, which is the point: the app opens one tap away from recording
/// something.
class AppShell extends StatefulWidget {
  const AppShell({
    super.key,
    required this.screenBuilder,
    required this.onNewExpense,
  });

  final Widget Function(BuildContext context, Destination destination)
      screenBuilder;

  /// Start entering an expense.
  final void Function(BuildContext context) onNewExpense;

  @override
  State<AppShell> createState() => AppShellState();
}

class AppShellState extends State<AppShell> {
  Destination _selected = Destination.home;

  /// Move to a destination from outside — the overview offering something the
  /// Budget screen owns, rather than a second copy of it.
  void show(Destination destination) {
    setState(() => _selected = destination);
  }

  void _select(int index) {
    setState(() {
      _selected = Destination.values[index];
    });
  }

  @override
  Widget build(BuildContext context) {
    final sizeClass = WindowSizeClass.of(MediaQuery.sizeOf(context).width);
    final body = widget.screenBuilder(context, _selected);
    final appBar = AppBar(title: Text(_selected.label));
    void newExpense() => widget.onNewExpense(context);

    return switch (sizeClass) {
      WindowSizeClass.compact => Scaffold(
          appBar: appBar,
          body: body,
          floatingActionButton: FloatingActionButton(
            onPressed: newExpense,
            tooltip: 'New expense',
            child: const Icon(Icons.add),
          ),
          bottomNavigationBar: NavigationBar(
            selectedIndex: _selected.index,
            onDestinationSelected: _select,
            destinations: [
              for (final d in Destination.values)
                NavigationDestination(
                  icon: Icon(d.icon),
                  selectedIcon: Icon(d.selectedIcon),
                  label: d.label,
                ),
            ],
          ),
        ),
      WindowSizeClass.medium => Scaffold(
          appBar: appBar,
          body: Row(
            children: [
              NavigationRail(
                selectedIndex: _selected.index,
                onDestinationSelected: _select,
                // The rail's leading slot is where Material 3 puts the primary
                // action.
                leading: FloatingActionButton(
                  onPressed: newExpense,
                  tooltip: 'New expense',
                  elevation: 0,
                  child: const Icon(Icons.add),
                ),
                // Labels under every icon, not only the selected one: five
                // destinations of similar weight, none of which is a mode the
                // user should have to hover to name.
                labelType: NavigationRailLabelType.all,
                destinations: [
                  for (final d in Destination.values)
                    NavigationRailDestination(
                      icon: Icon(d.icon),
                      selectedIcon: Icon(d.selectedIcon),
                      label: Text(d.label),
                    ),
                ],
              ),
              const VerticalDivider(width: 1, thickness: 1),
              Expanded(child: body),
            ],
          ),
        ),
      WindowSizeClass.expanded ||
      WindowSizeClass.large ||
      WindowSizeClass.extraLarge =>
        Scaffold(
          appBar: appBar,
          // A drawer has no slot for the primary action, so the button floats
          // over the content, which is where Material 3 puts it by default.
          floatingActionButton: FloatingActionButton.extended(
            onPressed: newExpense,
            icon: const Icon(Icons.add),
            label: const Text('New expense'),
          ),
          body: Row(
            children: [
              // A standard drawer, permanently on screen rather than opened
              // over the content — a modal drawer here would hide the thing
              // the extra width exists to show.
              NavigationDrawer(
                selectedIndex: _selected.index,
                onDestinationSelected: _select,
                children: [
                  Padding(
                    // The drawer's headline. 28dp of leading padding is the
                    // spec's, and it is what lines the text up with the
                    // destination labels rather than with their icons.
                    padding: const EdgeInsets.fromLTRB(28, 16, 16, 10),
                    child: Text(
                      'Personal Accounting',
                      style: Theme.of(context).textTheme.titleSmall?.copyWith(
                            color: Theme.of(context).colorScheme.onSurfaceVariant,
                          ),
                    ),
                  ),
                  for (final d in Destination.values)
                    NavigationDrawerDestination(
                      icon: Icon(d.icon),
                      selectedIcon: Icon(d.selectedIcon),
                      label: Text(d.label),
                    ),
                ],
              ),
              const VerticalDivider(width: 1, thickness: 1),
              Expanded(child: body),
            ],
          ),
        ),
    };
  }
}

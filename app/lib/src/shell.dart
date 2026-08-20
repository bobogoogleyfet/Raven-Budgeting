import 'package:flutter/material.dart';

import 'breakpoints.dart';
import 'destinations.dart';

/// The navigation shell.
///
/// Two things decide what is on screen. Which navigation component shows is a
/// function of the Material 3 window size class: a bottom bar when the window
/// is compact, a rail once there is room beside the content, a standard drawer
/// when there is room for labels next to icons. What fills the body is the
/// selected destination, or — when nothing is selected, which is how the app
/// starts — the entry form.
///
/// The form is not a destination. On a compact window that costs it the
/// navigation bar while it is up, because `NavigationBar` asserts a valid
/// selected index and has no "nothing selected" state; the rail and the drawer
/// both take `null`. That is the right answer rather than a workaround:
/// entering an expense on a phone is a focused task, and Material 3 gives a
/// focused task the whole screen.
class AppShell extends StatefulWidget {
  const AppShell({
    super.key,
    required this.homeTitle,
    required this.homeBuilder,
    required this.screenBuilder,
  });

  /// What the app bar says while the entry form is up.
  final String homeTitle;

  final WidgetBuilder homeBuilder;

  final Widget Function(BuildContext context, Destination destination)
      screenBuilder;

  @override
  State<AppShell> createState() => _AppShellState();
}

class _AppShellState extends State<AppShell> {
  /// `null` means the entry form — where the app starts.
  Destination? _selected;

  void _select(int index) {
    setState(() {
      _selected = Destination.values[index];
    });
  }

  void _goHome() {
    setState(() {
      _selected = null;
    });
  }

  /// Leaving the form without saving lands on Expenses, because a window that
  /// has just closed a form and shows nothing is a window that looks broken.
  void _leaveHome() {
    setState(() {
      _selected = Destination.expenses;
    });
  }

  @override
  Widget build(BuildContext context) {
    final sizeClass = WindowSizeClass.of(MediaQuery.sizeOf(context).width);
    final selected = _selected;
    final atHome = selected == null;

    final body =
        atHome ? widget.homeBuilder(context) : widget.screenBuilder(context, selected);
    final appBar = AppBar(
      title: Text(atHome ? widget.homeTitle : selected.label),
      actions: [
        if (atHome && sizeClass == WindowSizeClass.compact)
          IconButton(
            onPressed: _leaveHome,
            icon: const Icon(Icons.close),
            tooltip: 'Close',
          ),
      ],
    );

    return switch (sizeClass) {
      WindowSizeClass.compact => Scaffold(
          appBar: appBar,
          body: body,
          floatingActionButton: atHome
              ? null
              : FloatingActionButton(
                  onPressed: _goHome,
                  tooltip: widget.homeTitle,
                  child: const Icon(Icons.add),
                ),
          bottomNavigationBar: atHome
              ? null
              : NavigationBar(
                  selectedIndex: selected.index,
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
                selectedIndex: selected?.index,
                onDestinationSelected: _select,
                // The rail's leading slot is where Material 3 puts the primary
                // action, and entering an expense is the only one this app has.
                leading: FloatingActionButton(
                  onPressed: _goHome,
                  tooltip: widget.homeTitle,
                  elevation: 0,
                  child: const Icon(Icons.add),
                ),
                // Labels under every icon, not only the selected one: four
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
          floatingActionButton: atHome
              ? null
              : FloatingActionButton.extended(
                  onPressed: _goHome,
                  icon: const Icon(Icons.add),
                  label: Text(widget.homeTitle),
                ),
          body: Row(
            children: [
              // A standard drawer, permanently on screen rather than opened
              // over the content — a modal drawer here would hide the thing
              // the extra width exists to show.
              NavigationDrawer(
                selectedIndex: selected?.index,
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

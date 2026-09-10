import 'package:flutter/material.dart';

import 'breakpoints.dart';
import 'destinations.dart';
import 'fab.dart';

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
/// and a floating button in the lower right where there is not. It is on the
/// screens that are about money going out, which [Destination.offersNewExpense]
/// names: the app opens one tap away from recording something.
/// How much of the bottom of the content a screen has pinned something to.
///
/// The floating button is anchored to the bottom of the content area, so it
/// lands on anything a screen pins there. Flutter lifts the button over the
/// `Scaffold`'s own bottom slots and over nothing else, and those slots span
/// the whole window — the wrong width entirely once a rail or a drawer is
/// beside the content. So the screen measures what it pinned and says so, and
/// the shell lifts its button by that much.
///
/// Screens with nothing pinned say nothing, and the button sits where Material
/// 3 puts it.
class BottomLineInset extends InheritedWidget {
  const BottomLineInset({
    super.key,
    required this.inset,
    required super.child,
  });

  final ValueNotifier<double> inset;

  /// Read without subscribing.
  ///
  /// A screen only ever writes here; depending on the value it publishes would
  /// rebuild it every time it published, which is a loop with an extra frame
  /// in it.
  static ValueNotifier<double>? of(BuildContext context) => context
      .getInheritedWidgetOfExactType<BottomLineInset>()
      ?.inset;

  @override
  bool updateShouldNotify(BottomLineInset old) => inset != old.inset;
}

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

  /// What the screen on show has pinned to the bottom of the content.
  final ValueNotifier<double> _bottomLine = ValueNotifier<double>(0);

  @override
  void dispose() {
    _bottomLine.dispose();
    super.dispose();
  }

  /// Move to a destination from outside — the overview offering something the
  /// Budget screen owns, rather than a second copy of it.
  void show(Destination destination) {
    setState(() => _selected = destination);
  }

  void _select(int index) {
    setState(() {
      _selected = Destination.values[index];
      // The screen being left took its bottom line with it, and the one
      // arriving has not measured its own yet. Left as it was, the button
      // spends a frame lifted over a line that is no longer there.
      _bottomLine.value = 0;
    });
  }

  @override
  Widget build(BuildContext context) {
    final sizeClass = WindowSizeClass.of(MediaQuery.sizeOf(context).width);
    // Every screen is built inside this, so any of them may pin a bottom line
    // and have the button lifted over it without the shell knowing which did.
    final body = BottomLineInset(
      inset: _bottomLine,
      child: widget.screenBuilder(context, _selected),
    );
    final appBar = AppBar(title: Text(_selected.label));
    void newExpense() => widget.onNewExpense(context);
    final bool offersNewExpense = _selected.offersNewExpense;

    // Lifted clear of whatever the screen pinned under its content. Rebuilt on
    // its own rather than with the shell: the measurement arrives a frame late
    // by definition, and rebuilding five destinations for it would throw away
    // the scroll position of the list that just published it.
    Widget lifted(Widget button) => ValueListenableBuilder<double>(
          valueListenable: _bottomLine,
          builder: (context, inset, child) => Padding(
            padding: EdgeInsets.only(bottom: inset),
            child: child,
          ),
          child: button,
        );

    return switch (sizeClass) {
      WindowSizeClass.compact => Scaffold(
          appBar: appBar,
          body: body,
          // Medium, not the 56dp default: M3 asks for a medium FAB in mobile
          // layouts, and this is the only place the button still floats over
          // content, so it is the only place the size is ours to choose.
          floatingActionButton: offersNewExpense
              ? lifted(MediumFab(
                  onPressed: newExpense,
                  tooltip: 'New expense',
                  icon: Icons.add,
                ))
              : null,
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
                // action. On a screen that offers none the slot is held open
                // rather than dropped: leaving it out pulls all five
                // destinations 64dp up the rail, so navigating to Budget
                // moves the very control you would navigate back with. An
                // empty 56dp of rail is invisible; destinations that jump are
                // not.
                //
                // The 56dp size, not the medium one the compact window gets:
                // the rail is 80dp wide, so a medium FAB fills it edge to edge
                // with nothing either side. The size guidance is about mobile
                // layouts against tablet ones; the width of the navigation
                // component the button is sitting inside decides this one.
                leading: offersNewExpense
                    ? FloatingActionButton(
                        onPressed: newExpense,
                        tooltip: 'New expense',
                        elevation: 0,
                        child: const Icon(Icons.add),
                      )
                    : const SizedBox.square(dimension: 56),
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
          // The lower right, the same corner the compact window uses. Material
          // 3 suggests considering the upper left at these widths — inside the
          // navigation component — but says in the same breath to test whether
          // it is really the best position, and allows a FAB aligned left,
          // center or right. Extended, because there is width here for the
          // label to cost nothing.
          floatingActionButton: offersNewExpense
              ? lifted(FloatingActionButton.extended(
                  onPressed: newExpense,
                  icon: const Icon(Icons.add),
                  label: const Text('New expense'),
                ))
              : null,
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
                      'Raven Budgeting',
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

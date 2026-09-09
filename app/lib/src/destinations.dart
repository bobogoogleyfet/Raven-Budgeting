import 'package:flutter/material.dart';

/// The five screens you navigate to.
///
/// Entering an expense is not among them: it is a task rather than a place,
/// and it lives on a button that floats over the screens where recording one
/// is the obvious next thing. Home is where the app opens — the budget at a
/// glance, and what was spent most recently.
///
/// Each carries two icons. Material 3 states an unselected destination is
/// outlined and a selected one is filled — the change of weight is how the
/// selection reads at a glance, and using one icon for both states loses it.
enum Destination {
  home('Home', Icons.home_outlined, Icons.home),
  expenses('Expenses', Icons.receipt_long_outlined, Icons.receipt_long),
  inbox('Inbox', Icons.inbox_outlined, Icons.inbox),
  budget('Budget', Icons.pie_chart_outline, Icons.pie_chart, offersNewExpense: false),
  settings('Settings', Icons.settings_outlined, Icons.settings, offersNewExpense: false);

  const Destination(
    this.label,
    this.icon,
    this.selectedIcon, {
    this.offersNewExpense = true,
  });

  final String label;
  final IconData icon;
  final IconData selectedIcon;

  /// Whether the new-expense button floats over this screen.
  ///
  /// It does over the three that are about money already going out — the
  /// overview, the list, and the queue of things not yet categorized. It does
  /// not over Budget or Settings, which are where the plan is written rather
  /// than where it is spent against.
  ///
  /// Material 3 allows one primary action per screen, and Budget already has
  /// its own: "Set a budget" sits in that screen's header, and a second,
  /// heavier button offering something unrelated is the one Budget was *not*
  /// opened to do. Settings has no entry action at all, so the button there
  /// was only ever a thing to hit by accident.
  final bool offersNewExpense;
}

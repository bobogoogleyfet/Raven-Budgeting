import 'package:flutter/material.dart';

/// The five screens you navigate to.
///
/// Entering an expense is not among them: it is a task rather than a place,
/// and it lives on the button that floats over all five. Home is where the app
/// opens — the budget at a glance, and what was spent most recently.
///
/// Each carries two icons. Material 3 states an unselected destination is
/// outlined and a selected one is filled — the change of weight is how the
/// selection reads at a glance, and using one icon for both states loses it.
enum Destination {
  home('Home', Icons.home_outlined, Icons.home),
  expenses('Expenses', Icons.receipt_long_outlined, Icons.receipt_long),
  inbox('Inbox', Icons.inbox_outlined, Icons.inbox),
  budget('Budget', Icons.pie_chart_outline, Icons.pie_chart),
  settings('Settings', Icons.settings_outlined, Icons.settings);

  const Destination(this.label, this.icon, this.selectedIcon);

  final String label;
  final IconData icon;
  final IconData selectedIcon;
}

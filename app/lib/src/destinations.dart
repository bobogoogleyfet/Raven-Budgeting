import 'package:flutter/material.dart';

/// The four screens you navigate to.
///
/// Entering an expense is deliberately not among them. It is what the app
/// opens to, and a destination you are already standing in is a tab that can
/// never be used.
///
/// Each carries two icons. Material 3 states an unselected destination is
/// outlined and a selected one is filled — the change of weight is how the
/// selection reads at a glance, and using one icon for both states loses it.
enum Destination {
  expenses('Expenses', Icons.receipt_long_outlined, Icons.receipt_long),
  inbox('Inbox', Icons.inbox_outlined, Icons.inbox),
  budget('Budget', Icons.pie_chart_outline, Icons.pie_chart),
  settings('Settings', Icons.settings_outlined, Icons.settings);

  const Destination(this.label, this.icon, this.selectedIcon);

  final String label;
  final IconData icon;
  final IconData selectedIcon;
}

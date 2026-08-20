import 'dart:io';

import 'package:flutter/material.dart';

import 'src/destinations.dart';
import 'src/rust/api/db.dart';
import 'src/rust/frb_generated.dart';
import 'src/screens/budget_screen.dart';
import 'src/screens/entry_screen.dart';
import 'src/screens/expenses_screen.dart';
import 'src/screens/home_screen.dart';
import 'src/screens/placeholder_screen.dart';
import 'src/shell.dart';
import 'src/theme.dart';

Future<void> main() async {
  await RustLib.init();

  // Desktop resolves the path in Rust, from the same environment variables the
  // CLI reads. Android and iOS cannot: the per-app directory is the platform's
  // answer and only Dart can ask for it, so phase 6 passes `path_provider`'s
  // result to `openDatabase` here. Failing loudly now is better than silently
  // opening a database somewhere the platform will delete.
  if (Platform.isAndroid || Platform.isIOS) {
    throw UnsupportedError(
      'Mobile needs its data directory passed to openDatabase — phase 6.',
    );
  }

  runApp(AccountingApp(databasePath: await openDatabase()));
}

class AccountingApp extends StatelessWidget {
  const AccountingApp({super.key, required this.databasePath});

  final String databasePath;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Personal Accounting',
      theme: lightTheme(),
      darkTheme: darkTheme(),
      home: AppShell(screenBuilder: _screenFor, onNewExpense: _newExpense),
    );
  }

  Widget _screenFor(BuildContext context, Destination destination) {
    return switch (destination) {
      Destination.home => const HomeScreen(),
      Destination.expenses => const ExpensesScreen(),
      Destination.budget => const BudgetScreen(),
      Destination.inbox =>
        PlaceholderScreen(destination: destination, phase: 'phase 5'),
      Destination.settings => PlaceholderScreen(
          destination: destination,
          phase: 'phase 5',
          databasePath: databasePath,
        ),
    };
  }

  /// Entering an expense is a focused task, so it gets a screen of its own
  /// rather than a pane inside the shell — Material 3's full-screen dialog,
  /// which is also what makes it work identically on a phone.
  void _newExpense(BuildContext context) {
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        fullscreenDialog: true,
        builder: (context) => Scaffold(
          appBar: AppBar(title: const Text('New expense')),
          body: const EntryScreen(),
        ),
      ),
    );
  }
}

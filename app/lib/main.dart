import 'dart:io';

import 'package:flutter/material.dart';

import 'src/destinations.dart';
import 'dart:async';

import 'src/api.dart' show homeLocation, refreshRates;
import 'src/rust/api/db.dart';
import 'src/rust/frb_generated.dart';
import 'src/screens/budget_screen.dart';
import 'src/screens/entry_screen.dart';
import 'src/screens/expenses_screen.dart';
import 'src/screens/home_area_form.dart';
import 'src/screens/inbox_screen.dart';
import 'src/screens/settings_screen.dart';
import 'src/screens/home_screen.dart';
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

  final String path = await openDatabase();
  // First run asks where the user lives before anything else.
  //
  // Detected by the home location still carrying a 0% rate, which is what the
  // seed installs: no rate means every receipt would record $0.00 of tax, and
  // the app cannot guess the right one.
  //
  // A failure here is not worth blocking entry over — the entry form will
  // surface the real error when it calls the same core.
  bool configured = false;
  try {
    final home = await homeLocation();
    configured = home != null && home.taxRatePpm > 0;
  } catch (_) {
    configured = false;
  }

  // Rates change on the first day of a quarter, so this does something four
  // times a year and nothing on the other 361 days. Deliberately not awaited:
  // the app must open instantly on all of them, including the ones where the
  // network is a captive portal that will hold a connection open for thirty
  // seconds before failing. Nothing on screen is waiting for the result, and a
  // failure is not an error — being offline is a laptop's normal state.
  unawaited(() async {
    try {
      await refreshRates();
    } catch (_) {
      // Being offline is a laptop's normal state, not a fault.
    }
  }());

  runApp(AccountingApp(databasePath: path, configured: configured));
}

class AccountingApp extends StatefulWidget {
  const AccountingApp({
    super.key,
    required this.databasePath,
    required this.configured,
  });

  final String databasePath;

  /// Whether the app has been told where home is. Until it has, there is
  /// nothing worth showing: every receipt entered would record no tax.
  final bool configured;

  @override
  State<AccountingApp> createState() => _AccountingAppState();
}

class _AccountingAppState extends State<AccountingApp> {
  late bool _configured = widget.configured;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Personal Accounting',
      theme: lightTheme(),
      darkTheme: darkTheme(),
      home: _configured
          ? AppShell(screenBuilder: _screenFor, onNewExpense: _newExpense)
          : Scaffold(
              appBar: AppBar(title: const Text('Welcome')),
              body: HomeAreaForm(
                firstRun: true,
                submitLabel: 'Save and start',
                onSaved: () => setState(() => _configured = true),
              ),
            ),
    );
  }

  Widget _screenFor(BuildContext context, Destination destination) {
    return switch (destination) {
      Destination.home => const HomeScreen(),
      Destination.expenses => const ExpensesScreen(),
      Destination.budget => const BudgetScreen(),
      Destination.inbox => const InboxScreen(),
      Destination.settings => SettingsScreen(databasePath: widget.databasePath),
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


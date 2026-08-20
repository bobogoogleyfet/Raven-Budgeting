import 'dart:io';

import 'package:flutter/material.dart';

import 'src/destinations.dart';
import 'src/rust/api/db.dart';
import 'src/rust/frb_generated.dart';
import 'src/screens/categories_screen.dart';
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
      home: AppShell(
        homeTitle: 'New expense',
        homeBuilder: (context) => const CategoriesScreen(),
        screenBuilder: _screenFor,
      ),
    );
  }

  /// The entry form is phase 4. Until then the home surface shows the
  /// categories that form will be built out of, which is what makes phase 1
  /// worth running: the rows come from the real database through the real
  /// bridge.
  Widget _screenFor(BuildContext context, Destination destination) {
    return switch (destination) {
      Destination.expenses =>
        PlaceholderScreen(destination: destination, phase: 'phase 3'),
      Destination.inbox ||
      Destination.budget =>
        PlaceholderScreen(destination: destination, phase: 'phase 5'),
      Destination.settings => PlaceholderScreen(
          destination: destination,
          phase: 'phase 5',
          databasePath: databasePath,
        ),
    };
  }
}

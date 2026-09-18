import 'package:flutter/material.dart';

import '../theme.dart';

/// A whole app that says one thing: why Raven is not open.
///
/// Its own `MaterialApp` because it stands in for the real one before there is
/// any database behind it — a second tab waiting its turn, or a browser that
/// cannot be trusted with the data — and the real app cannot be built without
/// one. It borrows the real theme so the page is recognisably the same app,
/// and asks nothing of the bridge, so it can be shown before Rust has loaded
/// or after it failed to.
class BlockedApp extends StatelessWidget {
  const BlockedApp({
    super.key,
    required this.icon,
    required this.title,
    required this.message,
  });

  final IconData icon;
  final String title;
  final String message;

  /// Shown by a second tab while the first holds the database. It opens on
  /// its own as soon as the other tab closes, so there is nothing to press.
  const BlockedApp.otherTab({super.key})
      : icon = Icons.tab_outlined,
        title = 'Raven is open in another tab',
        message = 'Only one tab can use your data at a time, so this one is '
            'waiting. Close the other tab and this one opens by itself.';

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Raven Budgeting',
      debugShowCheckedModeBanner: false,
      theme: lightTheme(),
      darkTheme: darkTheme(),
      home: Builder(
        builder: (context) {
          final theme = Theme.of(context);
          return Scaffold(
            body: SafeArea(
              child: Center(
                child: SingleChildScrollView(
                  padding: const EdgeInsets.all(24),
                  child: ConstrainedBox(
                    constraints: const BoxConstraints(maxWidth: 560),
                    child: Semantics(
                      liveRegion: true,
                      child: Column(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          Icon(
                            icon,
                            size: 48,
                            color: theme.colorScheme.onSurfaceVariant,
                          ),
                          const SizedBox(height: 16),
                          Text(
                            title,
                            textAlign: TextAlign.center,
                            style: theme.textTheme.headlineSmall,
                          ),
                          const SizedBox(height: 8),
                          Text(
                            message,
                            textAlign: TextAlign.center,
                            style: theme.textTheme.bodyMedium?.copyWith(
                              color: theme.colorScheme.onSurfaceVariant,
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}

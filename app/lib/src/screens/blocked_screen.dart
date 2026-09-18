import 'package:flutter/material.dart';

import '../theme.dart';

/// A whole app that says one thing: why Raven is not open.
///
/// Its own `MaterialApp` because it stands in for the real one when there is
/// no database behind it — most often a second tab, turned away because
/// another already has the data open — and the real app cannot be built
/// without one. It borrows the real theme so the page is recognisably the same
/// app, and asks nothing of the bridge, so it can be shown before Rust has
/// loaded or after it failed to.
class BlockedApp extends StatelessWidget {
  const BlockedApp({
    super.key,
    required this.icon,
    required this.title,
    required this.message,
    this.actionLabel,
    this.onAction,
  });

  final IconData icon;
  final String title;
  final String message;

  /// The one thing worth pressing, when there is one.
  final String? actionLabel;
  final VoidCallback? onAction;

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
                          if (actionLabel != null && onAction != null) ...[
                            const SizedBox(height: 24),
                            FilledButton(
                              onPressed: onAction,
                              child: Text(actionLabel!),
                            ),
                          ],
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

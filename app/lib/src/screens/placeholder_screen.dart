import 'package:flutter/material.dart';

import '../destinations.dart';

/// A screen that does not exist yet.
///
/// Named for the phase that builds it, so an empty tab reads as work still to
/// do rather than as something broken.
class PlaceholderScreen extends StatelessWidget {
  const PlaceholderScreen({
    super.key,
    required this.destination,
    required this.phase,
    this.databasePath,
  });

  final Destination destination;
  final String phase;
  final String? databasePath;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              destination.icon,
              size: 48,
              color: theme.colorScheme.onSurfaceVariant,
            ),
            const SizedBox(height: 16),
            Text(destination.label, style: theme.textTheme.titleMedium),
            const SizedBox(height: 4),
            Text(
              'Arrives in $phase.',
              style: theme.textTheme.bodyMedium
                  ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
            ),
            if (databasePath != null) ...[
              const SizedBox(height: 24),
              Text('Database', style: theme.textTheme.labelMedium),
              const SizedBox(height: 4),
              SelectableText(
                databasePath!,
                textAlign: TextAlign.center,
                style: theme.textTheme.bodySmall
                    ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

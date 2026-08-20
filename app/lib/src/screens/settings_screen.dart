import 'package:flutter/material.dart';

import 'home_area_form.dart';

/// Settings: the handful of answers the app asked for once and then relied on.
///
/// Today that is the home area — the location every expense inherits unless it
/// names another one. It was asked at first run and there was no way back to
/// it, so a user who moved, or who picked the wrong town in a list of four
/// hundred, had no way to correct the rate every later expense was estimated
/// with.
class SettingsScreen extends StatelessWidget {
  const SettingsScreen({super.key, this.databasePath});

  final String? databasePath;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Expanded(
          child: HomeAreaForm(
            submitLabel: 'Save location',
            onSaved: () {},
          ),
        ),
        if (databasePath != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text('Database', style: theme.textTheme.labelMedium),
                const SizedBox(height: 4),
                SelectableText(
                  databasePath!,
                  style: theme.textTheme.bodySmall
                      ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
                ),
              ],
            ),
          ),
      ],
    );
  }
}

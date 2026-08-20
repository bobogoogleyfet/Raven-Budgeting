import 'package:flutter/material.dart';

import '../rust/api/categories.dart';

/// How the screen gets its rows. Defaults to the real bridge call; a test
/// passes its own so the widget can be exercised without loading the library.
typedef CategoryLoader = Future<List<CategoryDto>> Function();

/// The seeded categories, grouped as the core returns them.
///
/// The order is the core's — group sort order, then category sort order, then
/// name — and this screen does not re-sort it. Where the rows sit is a
/// property of the data, not of the rendering.
class CategoriesScreen extends StatefulWidget {
  const CategoriesScreen({super.key, this.load = listCategories});

  final CategoryLoader load;

  @override
  State<CategoriesScreen> createState() => _CategoriesScreenState();
}

class _CategoriesScreenState extends State<CategoriesScreen> {
  late Future<List<CategoryDto>> _categories;

  @override
  void initState() {
    super.initState();
    _categories = widget.load();
  }

  void _retry() {
    // A block body, not an arrow: an arrow returns the assignment's value,
    // which is a `Future`, and `setState` asserts against being handed one.
    setState(() {
      _categories = widget.load();
    });
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<List<CategoryDto>>(
      future: _categories,
      builder: (context, snapshot) {
        if (snapshot.hasError) {
          return _Failed(message: '${snapshot.error}', onRetry: _retry);
        }
        if (!snapshot.hasData) {
          return const Center(child: CircularProgressIndicator());
        }
        return _CategoryList(categories: snapshot.requireData);
      },
    );
  }
}

class _CategoryList extends StatelessWidget {
  const _CategoryList({required this.categories});

  final List<CategoryDto> categories;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    // One flat list of headers and rows, so the whole thing scrolls as a unit
    // and stays lazy.
    final rows = <Widget>[];
    String? group;
    for (final category in categories) {
      if (category.groupName != group) {
        group = category.groupName;
        rows.add(
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 24, 16, 8),
            child: Text(
              group,
              style: theme.textTheme.titleSmall
                  ?.copyWith(color: theme.colorScheme.primary),
            ),
          ),
        );
      }
      rows.add(
        ListTile(
          title: Text(category.name),
          trailing: category.isBudgeted
              ? Text('Budgeted', style: theme.textTheme.labelSmall)
              : null,
        ),
      );
    }

    return ListView.builder(
      itemCount: rows.length,
      itemBuilder: (context, index) => rows[index],
    );
  }
}

class _Failed extends StatelessWidget {
  const _Failed({required this.message, required this.onRetry});

  final String message;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(Icons.error_outline, color: theme.colorScheme.error),
            const SizedBox(height: 12),
            // The core writes its error messages to be read by a person, and
            // the bridge passes them through unchanged. Showing the message is
            // more use than replacing it with one of our own.
            Text(message, textAlign: TextAlign.center),
            const SizedBox(height: 16),
            FilledButton.tonal(onPressed: onRetry, child: const Text('Retry')),
          ],
        ),
      ),
    );
  }
}

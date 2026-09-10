import 'package:flutter/material.dart';

import '../api.dart';
import '../breakpoints.dart';
import '../fab.dart';
import '../widgets/money_text.dart';

typedef InboxLoader = Future<List<TxnSummary>> Function();
typedef CategoryLoader = Future<List<CategoryDto>> Function();
typedef Filer = Future<void> Function(int id, int categoryId);

/// The needs-categorizing queue.
///
/// `pacc quick-add 42.18` records a total in the checkout line and files it
/// under Uncategorized so the decision can happen later. This is later. The
/// amounts are already on disk and are not re-entered here — only which
/// envelope they came out of, which is why each row needs one picker and
/// nothing else.
class InboxScreen extends StatefulWidget {
  const InboxScreen({
    super.key,
    this.load = needsCategorizing,
    this.loadCategories = listCategories,
    this.file = categorizeWhole,
    this.onChanged,
  });

  final InboxLoader load;
  final CategoryLoader loadCategories;
  final Filer file;
  final VoidCallback? onChanged;

  @override
  State<InboxScreen> createState() => _InboxScreenState();
}

class _InboxScreenState extends State<InboxScreen> {
  List<CategoryDto> _categories = const [];
  List<TxnSummary> _rows = const [];
  bool _loading = true;
  String? _message;

  /// The category chosen per row, and the row currently being filed.
  final Map<int, int> _chosen = {};
  int? _filing;
  final Map<int, String> _problems = {};

  @override
  void initState() {
    super.initState();
    _boot();
  }

  Future<void> _boot() async {
    try {
      // Categories first and once: they are the same for every row, and every
      // refresh reuses them.
      _categories = await widget.loadCategories();
      await _refresh();
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _loading = false;
        _message = 'Could not load the inbox: $error';
      });
    }
  }

  Future<void> _refresh() async {
    final rows = await widget.load();
    if (!mounted) return;
    setState(() {
      _rows = rows;
      _loading = false;
      _message = rows.isEmpty
          ? 'Nothing waiting. Quick-added expenses show up here.'
          : null;
    });
  }

  Future<void> _fileIt(TxnSummary txn) async {
    final int? categoryId = _chosen[txn.id];
    // Belt and braces: the button is disabled until a category is chosen, and
    // sending 0 would reach the foreign key rather than the core's own "split
    // has no category" check.
    if (categoryId == null) return;

    setState(() {
      _filing = txn.id;
      _problems.remove(txn.id);
    });
    try {
      await widget.file(txn.id, categoryId);
      await _refresh();
      widget.onChanged?.call();
    } catch (error) {
      if (!mounted) return;
      setState(() => _problems[txn.id] = '$error');
    } finally {
      if (mounted) setState(() => _filing = null);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    if (_loading) {
      return const Center(child: CircularProgressIndicator());
    }

    // Space for the shell's floating button, sized to whichever button this
    // width actually gets — and nothing at medium, where it is in the rail.
    final double reserve =
        fabReserve(WindowSizeClass.of(MediaQuery.sizeOf(context).width));

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Expanded(
          child: ListView.separated(
            padding: EdgeInsets.only(bottom: reserve),
            itemCount: _rows.length,
            separatorBuilder: (context, index) => const Divider(height: 1),
            itemBuilder: (context, index) {
              final txn = _rows[index];
              return _InboxRow(
                txn: txn,
                categories: _categories,
                chosen: _chosen[txn.id],
                busy: _filing == txn.id,
                problem: _problems[txn.id],
                onChoose: (id) => setState(() => _chosen[txn.id] = id),
                onFile: () => _fileIt(txn),
              );
            },
          ),
        ),
        if (_message != null)
          Semantics(
            liveRegion: true,
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Text(
                _message!,
                style: theme.textTheme.bodyMedium
                    ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
              ),
            ),
          ),
      ],
    );
  }
}

class _InboxRow extends StatelessWidget {
  const _InboxRow({
    required this.txn,
    required this.categories,
    required this.chosen,
    required this.busy,
    required this.problem,
    required this.onChoose,
    required this.onFile,
  });

  final TxnSummary txn;
  final List<CategoryDto> categories;
  final int? chosen;
  final bool busy;
  final String? problem;
  final ValueChanged<int> onChoose;
  final VoidCallback onFile;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              // The core's own date string, printed as it arrived.
              SizedBox(width: 110, child: Text(txn.txnDate)),
              Expanded(child: Text(txn.merchantName ?? '—')),
              MoneyText(txn.total, style: theme.textTheme.titleMedium),
            ],
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              Expanded(
                child: DropdownButtonFormField<int>(
                  initialValue: chosen,
                  isExpanded: true,
                  decoration: const InputDecoration(
                    isDense: true,
                    border: OutlineInputBorder(),
                  ),
                  hint: const Text('Choose a category…'),
                  items: [
                    for (final category in categories)
                      DropdownMenuItem<int>(
                        value: category.id,
                        child: Text('${category.groupName} / ${category.name}'),
                      ),
                  ],
                  onChanged: busy ? null : (id) => id == null ? null : onChoose(id),
                ),
              ),
              const SizedBox(width: 12),
              FilledButton(
                // Locked until a category is chosen: there is no sensible
                // default, and a receipt filed under the wrong envelope is
                // worse than one still waiting.
                onPressed: chosen == null || busy ? null : onFile,
                child: const Text('File it'),
              ),
            ],
          ),
          if (problem != null)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(
                problem!,
                style: theme.textTheme.bodySmall
                    ?.copyWith(color: theme.colorScheme.error),
              ),
            ),
        ],
      ),
    );
  }
}

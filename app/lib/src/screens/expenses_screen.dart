import 'package:flutter/material.dart';

import '../api.dart';
import '../money.dart';
import '../breakpoints.dart';
import '../widgets/filter_menu.dart';
import '../widgets/money_text.dart';

/// How many expenses the list asks for.
///
/// A hard cap with no pagination, as in the TypeScript UI. The 201st expense
/// is simply absent, and the list says so rather than pretending otherwise —
/// see [_Footnote].
const int _howMany = 200;

/// Stands for expenses with no store, which is what quick-add leaves behind.
///
/// A sentinel option value, not an id: it is filtered out before ids are sent
/// and folded into `includeNoMerchant` instead. Parsing it as a number would
/// give a merchant id of zero, or throw.
const String _noStore = 'none';

typedef ExpenseLoader = Future<List<TxnSummary>> Function(TxnQuery query);
typedef FilterValuesLoader = Future<FilterValues> Function();
typedef ExpenseDeleter = Future<void> Function(int id);

/// The expense list. Everything entered here or with `pacc add`.
class ExpensesScreen extends StatefulWidget {
  const ExpensesScreen({
    super.key,
    this.load = listTransactions,
    this.loadFilterValues = filterValues,
    this.delete = deleteTransaction,
  });

  final ExpenseLoader load;
  final FilterValuesLoader loadFilterValues;
  final ExpenseDeleter delete;

  @override
  State<ExpensesScreen> createState() => _ExpensesScreenState();
}

class _ExpensesScreenState extends State<ExpensesScreen> {
  /// Newest first, and the caret in the Date header flips it.
  ///
  /// Only Date sorts. A list of expenses is read chronologically; the other
  /// columns are what you narrow by, which is a different question and gets a
  /// different control.
  TxnSort _sort = TxnSort.dateDesc;

  List<FilterOption> _stores = const [];
  List<FilterOption> _categories = const [];
  Set<String> _hiddenStores = <String>{};
  Set<String> _hiddenCategories = <String>{};

  List<TxnSummary> _rows = const [];
  bool _loading = true;
  String? _message;

  @override
  void initState() {
    super.initState();
    // Menus first, so a list that arrives fast is never briefly filterable by
    // nothing — but the list loads either way, because a menu that failed to
    // fill still leaves the list usable.
    _loadMenus().whenComplete(_load);
  }

  /// What to ask for, or `null` when the answer needs no asking.
  TxnQuery? _query() {
    final stores = narrowedTo(_stores.map((o) => o.value).toList(), _hiddenStores);
    final categories =
        narrowedTo(_categories.map((o) => o.value).toList(), _hiddenCategories);

    // Everything hidden asks for nothing. The core reads an empty set as "no
    // filter", so that question is answered here rather than sent — a "Hide
    // all" that showed the whole table would be the opposite of itself.
    if (stores?.isEmpty == true || categories?.isEmpty == true) return null;

    return TxnQuery(
      limit: _howMany,
      merchantIds: stores
          ?.where((value) => value != _noStore)
          .map(int.parse)
          .toList(),
      includeNoMerchant: stores == null || stores.contains(_noStore),
      categoryIds: categories?.map(int.parse).toList(),
      sort: _sort,
    );
  }

  bool get _filtered => _hiddenStores.isNotEmpty || _hiddenCategories.isNotEmpty;

  Future<void> _load() async {
    final asked = _query();
    if (asked == null) {
      setState(() {
        _rows = const [];
        _loading = false;
        _message = 'No expenses match those filters.';
      });
      return;
    }

    setState(() => _loading = true);
    try {
      final rows = await widget.load(asked);
      if (!mounted) return;
      setState(() {
        _rows = rows;
        _loading = false;
        _message = rows.isEmpty
            ? (_filtered
                // Worded by case: an empty database and an over-narrow filter
                // look identical on screen and need different things done
                // about them.
                ? 'No expenses match those filters.'
                : 'No expenses yet. Add one with the button.')
            : null;
      });
    } catch (error) {
      if (!mounted) return;
      // The rows that were there stay there. Replacing a list you were reading
      // with an error page loses the thing you were reading.
      setState(() {
        _loading = false;
        _message = 'Could not load expenses: $error';
      });
    }
  }

  /// Fill the menus from what the columns actually hold.
  ///
  /// Re-read after a delete as well as at startup: removing the last expense
  /// from a shop should take that shop out of the menu, or the list offers a
  /// filter that can only ever match nothing.
  Future<void> _loadMenus() async {
    try {
      final values = await widget.loadFilterValues();
      if (!mounted) return;
      setState(() {
        _stores = [
          for (final m in values.merchants)
            FilterOption(value: '${m.id}', label: m.name),
          // Quick-added expenses have no store, and without a box of their own
          // they would vanish the moment the filter was touched. Offered only
          // when there is one, so the menu never lists a row matching nothing.
          if (values.anyWithoutMerchant)
            const FilterOption(value: _noStore, label: '(No store)'),
        ];
        _categories = [
          for (final c in values.categories)
            FilterOption(value: '${c.id}', label: c.name),
        ];
        _hiddenStores = pruned(_hiddenStores, _stores);
        _hiddenCategories = pruned(_hiddenCategories, _categories);
      });
    } catch (_) {
      // A menu that failed to fill still leaves the list usable, and the
      // list's own error reporting is the one that matters.
    }
  }

  void _flipSort() {
    setState(() {
      _sort = _sort == TxnSort.dateDesc ? TxnSort.dateAsc : TxnSort.dateDesc;
    });
    _load();
  }

  Future<void> _confirmDelete(TxnSummary txn) async {
    final bool confirmed = await showDialog<bool>(
          context: context,
          builder: (context) => _ConfirmDelete(txn: txn),
        ) ??
        false;
    if (!confirmed || !mounted) return;

    try {
      await widget.delete(txn.id);
      if (!mounted) return;
      // Removed from the list rather than reloading it: a reload would scroll
      // the user back to the top of two hundred rows to show them one that is
      // gone.
      setState(() => _rows = _rows.where((r) => r.id != txn.id).toList());
      await _loadMenus();
      // Except when it was the last one — then the list has something new to
      // say, and only a reload knows which message it is.
      if (mounted && _rows.isEmpty) await _load();
    } catch (error) {
      if (!mounted) return;
      setState(() => _message = 'Could not delete that expense: $error');
    }
  }

  @override
  Widget build(BuildContext context) {
    final sizeClass = WindowSizeClass.of(MediaQuery.sizeOf(context).width);
    final bool table = sizeClass != WindowSizeClass.compact;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _Controls(
          sort: _sort,
          onFlipSort: _flipSort,
          stores: _stores,
          categories: _categories,
          hiddenStores: _hiddenStores,
          hiddenCategories: _hiddenCategories,
          onStoresChanged: (hidden) {
            setState(() => _hiddenStores = hidden);
            _load();
          },
          onCategoriesChanged: (hidden) {
            setState(() => _hiddenCategories = hidden);
            _load();
          },
          table: table,
        ),
        const Divider(height: 1),
        if (_loading && _rows.isEmpty)
          const Expanded(child: Center(child: CircularProgressIndicator()))
        else
          Expanded(
            child: ListView.separated(
              itemCount: _rows.length,
              separatorBuilder: (context, index) => const Divider(height: 1),
              itemBuilder: (context, index) => _ExpenseRow(
                txn: _rows[index],
                table: table,
                onDelete: () => _confirmDelete(_rows[index]),
              ),
            ),
          ),
        if (_message != null)
          // A live region, so an empty result or a failure is announced rather
          // than only drawn. Under the list, not above it: it is what the list
          // has to say when the rows run out, and a message above the header
          // pushes everything down every time a filter matches nothing.
          Semantics(
            liveRegion: true,
            child: Padding(
              padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
              child: Text(
                _message!,
                style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                      color: Theme.of(context).colorScheme.onSurfaceVariant,
                    ),
              ),
            ),
          ),
        if (_rows.length == _howMany) const _Footnote(),
      ],
    );
  }
}

/// Says that the list stops, because a list that silently stops is one you
/// trust wrongly.
class _Footnote extends StatelessWidget {
  const _Footnote();

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 8, 16, 12),
      child: Text(
        'Showing the most recent $_howMany.',
        style: theme.textTheme.bodySmall
            ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
      ),
    );
  }
}

/// The sort control and the two filter menus.
///
/// One row of controls at every width rather than header cells on a desktop
/// and something else on a phone: the rules they carry are fiddly enough
/// written once.
class _Controls extends StatelessWidget {
  const _Controls({
    required this.sort,
    required this.onFlipSort,
    required this.stores,
    required this.categories,
    required this.hiddenStores,
    required this.hiddenCategories,
    required this.onStoresChanged,
    required this.onCategoriesChanged,
    required this.table,
  });

  final TxnSort sort;
  final VoidCallback onFlipSort;
  final List<FilterOption> stores;
  final List<FilterOption> categories;
  final Set<String> hiddenStores;
  final Set<String> hiddenCategories;
  final void Function(Set<String>) onStoresChanged;
  final void Function(Set<String>) onCategoriesChanged;
  final bool table;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final bool newestFirst = sort == TxnSort.dateDesc;

    final Widget sortButton = TextButton(
      onPressed: onFlipSort,
      // The title and the caret are one control, so the whole thing sorts
      // rather than an eight-pixel triangle being the only target.
      style: TextButton.styleFrom(
        minimumSize: table ? const Size.fromHeight(40) : null,
        padding: const EdgeInsets.symmetric(horizontal: 12),
        alignment: table ? Alignment.centerLeft : null,
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text('Date', style: theme.textTheme.labelLarge),
          const SizedBox(width: 4),
          Icon(
            newestFirst ? Icons.arrow_drop_down : Icons.arrow_drop_up,
            size: 18,
          ),
        ],
      ),
    );

    return Tooltip(
      message: newestFirst ? 'Newest first' : 'Oldest first',
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        child: Row(
          children: [
            if (table) ...[
              SizedBox(width: 132, child: sortButton),
              Expanded(
                flex: 3,
                child: FilterMenuButton(
                  label: 'Store',
                  options: stores,
                  hidden: hiddenStores,
                  onChanged: onStoresChanged,
                ),
              ),
              Expanded(
                flex: 4,
                child: FilterMenuButton(
                  label: 'Categories',
                  options: categories,
                  hidden: hiddenCategories,
                  onChanged: onCategoriesChanged,
                ),
              ),
              SizedBox(
                width: 140,
                child: Text(
                  'Total',
                  textAlign: TextAlign.right,
                  style: theme.textTheme.labelLarge,
                ),
              ),
              // Unlabelled: a header reading "Delete" over a column of delete
              // buttons says the same word twice.
              const SizedBox(width: 48),
            ] else ...[
              sortButton,
              FilterMenuButton(
                label: 'Store',
                options: stores,
                hidden: hiddenStores,
                onChanged: onStoresChanged,
                dense: true,
              ),
              FilterMenuButton(
                label: 'Categories',
                options: categories,
                hidden: hiddenCategories,
                onChanged: onCategoriesChanged,
                dense: true,
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _ExpenseRow extends StatelessWidget {
  const _ExpenseRow({
    required this.txn,
    required this.table,
    required this.onDelete,
  });

  final TxnSummary txn;
  final bool table;
  final VoidCallback onDelete;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    // What quick-add leaves behind, and what the Inbox is for.
    final Color? background = txn.needsReview
        ? theme.colorScheme.tertiaryContainer.withValues(alpha: 0.4)
        : null;

    final Widget deleteButton = IconButton(
      onPressed: onDelete,
      // Stroked rather than filled: a solid bin in a table of numbers pulls
      // the eye to the one control nobody should be drawn to.
      icon: const Icon(Icons.delete_outline),
      iconSize: 20,
      color: theme.colorScheme.onSurfaceVariant,
      tooltip: 'Delete this expense',
    );

    if (!table) {
      return Container(
        color: background,
        child: ListTile(
          title: Text(txn.merchantName ?? '—'),
          subtitle: Text(
            txn.categories.isEmpty
                ? txn.txnDate
                : '${txn.txnDate} · ${txn.categories.join(', ')}',
            overflow: TextOverflow.ellipsis,
          ),
          trailing: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              MoneyText(txn.total, style: theme.textTheme.titleMedium),
              deleteButton,
            ],
          ),
        ),
      );
    }

    return Container(
      color: background,
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
      child: Row(
        children: [
          // The core's own `YYYY-MM-DD`, printed as it arrived: parsing it into
          // a date and formatting it again is how a UTC shift gets
          // reintroduced on a screen that never needed a timezone.
          SizedBox(width: 132, child: Text(txn.txnDate)),
          Expanded(flex: 3, child: Text(txn.merchantName ?? '—')),
          Expanded(
            flex: 4,
            child: Text(
              txn.categories.join(', '),
              overflow: TextOverflow.ellipsis,
            ),
          ),
          SizedBox(
            width: 140,
            child: MoneyText(
              txn.total,
              textAlign: TextAlign.right,
              style: theme.textTheme.bodyLarge,
            ),
          ),
          SizedBox(width: 48, child: deleteButton),
        ],
      ),
    );
  }
}

/// A click asks, a modal answers.
///
/// The dialog names the expense — its date, store and total — because "are you
/// sure?" is not a question anyone can answer about a row they may no longer
/// be looking at. There is no undo behind this, which is exactly why the
/// asking is worth a modal rather than something dismissible by accident.
class _ConfirmDelete extends StatelessWidget {
  const _ConfirmDelete({required this.txn});

  final TxnSummary txn;

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('Delete this expense?'),
      content: Text(
        '${txn.txnDate} · ${txn.merchantName ?? 'no store'} · '
        '${formatMoney(txn.total)}',
      ),
      actions: [
        // Cancel first and focused: for a destructive action the accident
        // should be the harmless one. Dismissing the dialog any other way —
        // Escape, tapping outside — also resolves to false.
        TextButton(
          autofocus: true,
          onPressed: () => Navigator.of(context).pop(false),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(true),
          style: FilledButton.styleFrom(
            backgroundColor: Theme.of(context).colorScheme.error,
            foregroundColor: Theme.of(context).colorScheme.onError,
          ),
          // Named for the action, never "OK".
          child: const Text('Delete Expense'),
        ),
      ],
    );
  }
}

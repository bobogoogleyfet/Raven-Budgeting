import 'package:flutter/material.dart';

import '../api.dart';
import '../money.dart';
import '../breakpoints.dart';
import '../widgets/filter_menu.dart';
import '../widgets/money_text.dart';

/// How many expenses one page holds.
///
/// Paged in SQL rather than capped: the list used to ask for the most recent
/// two hundred and admit in a footnote that the two hundred and first was
/// simply absent. Fifty is about a screenful and a half at desktop widths, so
/// Next is a deliberate press rather than the only way to see anything.
const int _pageSize = 50;

/// Stands for expenses with no store, which is what quick-add leaves behind.
///
/// A sentinel option value, not an id: it is filtered out before ids are sent
/// and folded into `includeNoMerchant` instead. Parsing it as a number would
/// give a merchant id of zero, or throw.
const String _noStore = 'none';

typedef ExpenseLoader = Future<List<TxnSummary>> Function(TxnQuery query);
typedef ExpenseTotalsLoader = Future<TxnTotals> Function(TxnQuery query);
typedef FilterValuesLoader = Future<FilterValues> Function();
typedef ExpenseDeleter = Future<void> Function(int id);

/// The expense list. Everything entered here or with `pacc add`.
class ExpensesScreen extends StatefulWidget {
  const ExpensesScreen({
    super.key,
    this.load = listTransactions,
    this.loadTotals = transactionTotals,
    this.loadFilterValues = filterValues,
    this.delete = deleteTransaction,
  });

  final ExpenseLoader load;
  final ExpenseTotalsLoader loadTotals;
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

  /// Which page is on screen, counting from zero.
  ///
  /// Reset to zero by anything that changes what the filter matches. Keeping
  /// it would leave the reader on page 4 of a filter with one page in it,
  /// looking at nothing and with no clue why.
  int _page = 0;

  /// The whole filtered set, added up by the core. `null` until it arrives,
  /// and while the filter matches nothing.
  TxnTotals? _totals;

  /// Which load is the current one.
  ///
  /// Pressing Next twice quickly puts two requests in flight, and the slower
  /// one may land last. Without this the screen would settle on the page the
  /// user has already left.
  int _request = 0;

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
      limit: _pageSize,
      offset: _page * _pageSize,
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
        _totals = null;
        _loading = false;
        _message = 'No expenses match those filters.';
      });
      return;
    }

    final int mine = ++_request;
    setState(() => _loading = true);
    try {
      // Both at once: the page and the figure under it describe the same
      // filter, and fetching them one after the other would show a total for
      // the previous filter for as long as the second call took.
      final results = await Future.wait([
        widget.load(asked),
        widget.loadTotals(asked),
      ]);
      if (!mounted || mine != _request) return;
      final rows = results[0] as List<TxnSummary>;
      setState(() {
        _rows = rows;
        _totals = results[1] as TxnTotals;
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
      if (!mounted || mine != _request) return;
      // The rows that were there stay there. Replacing a list you were reading
      // with an error page loses the thing you were reading.
      setState(() {
        _loading = false;
        _message = 'Could not load expenses: $error';
      });
    }
  }

  /// Go to a page, from zero. Out-of-range presses are not possible: the
  /// controls that would ask for one are disabled instead.
  void _goToPage(int page) {
    if (page == _page) return;
    setState(() => _page = page);
    _load();
  }

  /// Back to the first page, because the filter now matches something else.
  ///
  /// Every filter and sort change goes through here. Narrowing to one shop
  /// while on page 4 would otherwise land on a page past the end of the new
  /// result — an empty screen that looks like the filter matched nothing.
  void _refilter(VoidCallback change) {
    setState(() {
      change();
      _page = 0;
    });
    _load();
  }

  /// Ask the core for the bottom line again, leaving the rows alone.
  ///
  /// Used after a delete, which changes what the filter sums without changing
  /// which page is on screen.
  Future<void> _reloadTotals() async {
    final asked = _query();
    if (asked == null) return;
    try {
      final totals = await widget.loadTotals(asked);
      if (!mounted) return;
      setState(() => _totals = totals);
    } catch (_) {
      // Left as it was rather than blanked. The list is still readable, and
      // the delete it followed has already been reported on its own terms.
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
    // Back to page 1: page 4 of newest-first and page 4 of oldest-first hold
    // different receipts, and staying put would look like the sort had
    // scrambled the list rather than reversed it.
    _refilter(() {
      _sort = _sort == TxnSort.dateDesc ? TxnSort.dateAsc : TxnSort.dateDesc;
    });
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
      // the user back to the top of the page to show them a row that is gone.
      setState(() => _rows = _rows.where((r) => r.id != txn.id).toList());
      // The totals are reloaded even though the page is not. They are the sum
      // of the whole filter, so a deleted expense is still in them until they
      // are asked again — a bottom line that quietly disagrees with the rows
      // above it is worse than a moment of loading.
      await _reloadTotals();
      await _loadMenus();
      if (!mounted) return;
      if (_rows.isEmpty) {
        // The page has nothing left on it. On page 1 that is a list with
        // something new to say, and only a reload knows which message it is;
        // later on it means the last row of the last page went, so step back
        // rather than leaving the reader staring at an empty page they cannot
        // tell from a broken one.
        if (_page > 0) {
          _goToPage(_page - 1);
        } else {
          await _load();
        }
      }
    } catch (error) {
      if (!mounted) return;
      setState(() => _message = 'Could not delete that expense: $error');
    }
  }

  @override
  Widget build(BuildContext context) {
    final sizeClass = WindowSizeClass.of(MediaQuery.sizeOf(context).width);
    final bool table = sizeClass != WindowSizeClass.compact;
    final TxnTotals? totals = _totals;

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
          onStoresChanged: (hidden) =>
              _refilter(() => _hiddenStores = hidden),
          onCategoriesChanged: (hidden) =>
              _refilter(() => _hiddenCategories = hidden),
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
        // Pinned under the list rather than scrolled with it: a bottom line
        // you have to reach the end of the rows to see is one nobody reads.
        // Hidden entirely when nothing matched — a row of zeroes under a
        // message saying there is nothing is the same news twice.
        if (totals != null && totals.expenses > 0) ...[
          const Divider(height: 1),
          _TotalsRow(totals: totals, table: table),
          _Pager(
            first: _page * _pageSize + 1,
            last: _page * _pageSize + _rows.length,
            of: totals.expenses,
            onPrevious: _page == 0 ? null : () => _goToPage(_page - 1),
            // Disabled rather than hidden at the last page: a control that
            // disappears moves the one next to it under the pointer.
            onNext: _page * _pageSize + _rows.length >= totals.expenses
                ? null
                : () => _goToPage(_page + 1),
          ),
        ],
      ],
    );
  }
}

/// The bottom line: what the filter comes to, and over how many expenses.
///
/// **This totals the whole filtered set, not the page on screen.** That is the
/// useful number — "what have I spent at these shops" — and it deliberately
/// does not move when the reader presses Next. A per-page total would change
/// with the page size, which is an implementation detail of this screen, and it
/// would look for all the world like a bug in the arithmetic.
///
/// The figure is the core's, summed in SQL over every matching row. Nothing
/// here adds money: [Cents] makes that a compile error on purpose.
class _TotalsRow extends StatelessWidget {
  const _TotalsRow({required this.totals, required this.table});

  final TxnTotals totals;
  final bool table;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    // Said in full, because "Total" alone over a paged list is ambiguous in
    // exactly the way that matters: the reader cannot tell whether it means
    // this page or all of it.
    final String label = totals.expenses == 1
        ? 'Total of 1 expense'
        : 'Total of all ${totals.expenses} expenses';

    final Widget money = MoneyText(
      totals.total,
      textAlign: TextAlign.right,
      style: theme.textTheme.titleMedium
          ?.copyWith(fontWeight: FontWeight.w600),
    );

    return Container(
      // A container tone rather than a border: M3 separates a summary from the
      // rows it summarises by elevation, and the Divider above already draws
      // the line.
      color: theme.colorScheme.surfaceContainerHigh,
      padding: EdgeInsets.symmetric(horizontal: table ? 8 : 16, vertical: 12),
      child: Semantics(
        container: true,
        child: table
            ? Row(
                children: [
                  // Spans the date, store and category columns: none of them
                  // has a total, and three empty cells before the label would
                  // read as missing figures.
                  Expanded(
                    child: Text(label, style: theme.textTheme.labelLarge),
                  ),
                  // The same 140 as the rows above, so the money lines up
                  // under the money it adds up.
                  SizedBox(width: 140, child: money),
                  // Matches the delete column, which has nothing to total.
                  const SizedBox(width: 48),
                ],
              )
            : Row(
                children: [
                  Expanded(
                    child: Text(label, style: theme.textTheme.labelLarge),
                  ),
                  money,
                ],
              ),
      ),
    );
  }
}

/// Which slice of the result is on screen, and the way to the next one.
///
/// The range is spelled out rather than left as a page number: "showing 51-100
/// of 199" answers both "where am I" and "how much is there", and a bare "page
/// 2" answers neither.
class _Pager extends StatelessWidget {
  const _Pager({
    required this.first,
    required this.last,
    required this.of,
    required this.onPrevious,
    required this.onNext,
  });

  final int first;
  final int last;
  final int of;
  final VoidCallback? onPrevious;
  final VoidCallback? onNext;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 4, 8, 4),
      child: Row(
        children: [
          Expanded(
            child: Text(
              // An en dash, and no spaces around it: it is a range, not a
              // subtraction.
              'Showing $first\u2013$last of $of',
              style: theme.textTheme.bodySmall
                  ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
            ),
          ),
          IconButton(
            onPressed: onPrevious,
            icon: const Icon(Icons.chevron_left),
            tooltip: 'Previous page',
          ),
          IconButton(
            onPressed: onNext,
            icon: const Icon(Icons.chevron_right),
            tooltip: 'Next page',
          ),
        ],
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

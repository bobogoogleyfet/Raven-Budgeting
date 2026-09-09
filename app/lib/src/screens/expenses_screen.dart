import 'package:flutter/material.dart';

import '../api.dart';
import '../dates.dart';
import '../money.dart';
import '../breakpoints.dart';
import '../shell.dart';
import '../widgets/filter_menu.dart';
import '../widgets/money_text.dart';

/// The most rows one month may put on screen.
///
/// The month is the paging now, and a month is a human-sized amount of
/// shopping, so this is a guard rather than a page size: it exists so a
/// database that somehow holds thousands of receipts in one month cannot be
/// asked to render all of them at once. When it does bite, the list says so —
/// the old two-hundred cap was silent, and a ledger that quietly leaves rows
/// out is one you cannot trust the rest of.
const int _monthLimit = 500;

/// Blank space kept under the last row for the shell's floating button.
///
/// 56dp of button — the same height circular or extended — plus the 16dp it
/// floats above the edge. On the list rather than under the bottom line: the
/// bottom line is pinned *below* the button now, and it is the rows that need
/// somewhere to scroll to that the button is not sitting on.
///
/// Reserved only where the button actually floats over the content. At medium
/// widths it lives in the navigation rail, and a gap at the end of the list
/// there really would be a gap with nothing in it.
const double _fabReserve = 72;

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
///
/// # One month at a time
///
/// The list used to page fifty rows at a time, which answered "what is the
/// next fifty" — a question nobody asks about their own spending. A month is
/// the period the rest of the app already thinks in: budgets are monthly, the
/// overview is monthly, and "what did August come to" is the question this
/// screen is for. So the month control *is* the pagination, and it steps and
/// reads exactly as the Budget screen's does.
class ExpensesScreen extends StatefulWidget {
  const ExpensesScreen({
    super.key,
    this.load = listTransactions,
    this.loadTotals = transactionTotals,
    this.loadFilterValues = filterValues,
    this.delete = deleteTransaction,
    this.month,
  });

  final ExpenseLoader load;
  final ExpenseTotalsLoader loadTotals;
  final FilterValuesLoader loadFilterValues;
  final ExpenseDeleter delete;

  /// Which `YYYY-MM` to open on. Defaults to the month it is now.
  final String? month;

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

  /// The month on screen, and the whole of the paging.
  late String _month = widget.month ?? thisMonth();

  List<FilterOption> _stores = const [];
  List<FilterOption> _categories = const [];
  Set<String> _hiddenStores = <String>{};
  Set<String> _hiddenCategories = <String>{};

  List<TxnSummary> _rows = const [];

  /// The month, added up by the core, with the filters applied. `null` until
  /// it arrives, and while the filter matches nothing.
  TxnTotals? _totals;

  /// The pinned bottom line, so its height can be read after it is laid out.
  final GlobalKey _bottomLineKey = GlobalKey();

  /// Tell the shell how much of the bottom this screen has taken.
  ///
  /// Measured rather than computed: the row is one line of text at one text
  /// scale and two at another, and a number worked out here would be a second
  /// opinion about a height the framework has already settled. Runs after the
  /// frame that laid it out, which is the first moment there is anything to
  /// measure — so the button is in its unlifted place for that one frame,
  /// before there is a bottom line under it to land on.
  void _publishBottomLine() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || _inset == null) return;
      final box =
          _bottomLineKey.currentContext?.findRenderObject() as RenderBox?;
      _inset!.value = box?.hasSize == true ? box!.size.height : 0;
    });
  }

  /// Held rather than looked up once a frame.
  ///
  /// Nothing is published from `dispose`: the screen going away is the shell's
  /// own business and it zeroes the inset when it changes destination. A
  /// screen cannot reach its ancestors while it is being taken off the tree,
  /// and the notifier may already be gone with them.
  ValueNotifier<double>? _inset;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _inset = BottomLineInset.of(context);
  }

  /// Which load is the current one.
  ///
  /// Stepping the month twice quickly puts two requests in flight, and the
  /// slower one may land last. Without this the screen would settle on the
  /// month the user has already left.
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

    final range = monthRange(_month);

    return TxnQuery(
      limit: _monthLimit,
      // The bounds go to the totals as well as to the rows, so the bottom line
      // covers the month it sits under rather than the whole database.
      from: range.from,
      to: range.to,
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

  /// True when the month holds more expenses than the list asked for.
  ///
  /// Read off the core's own count rather than off [_monthLimit], so it is the
  /// fact that rows are missing that is reported, not a guess from the size of
  /// the request.
  bool get _truncated {
    final TxnTotals? totals = _totals;
    return totals != null && _rows.length < totals.expenses;
  }

  Future<void> _load() async {
    final asked = _query();
    if (asked == null) {
      setState(() {
        _rows = const [];
        _totals = null;
        _loading = false;
        _message = 'No expenses match those filters in ${monthLabel(_month)}.';
      });
      return;
    }

    final int mine = ++_request;
    setState(() => _loading = true);
    try {
      // Both at once: the rows and the figure under them describe the same
      // month and the same filter, and fetching them one after the other would
      // show last month's total for as long as the second call took.
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
                // Worded by case: a month with nothing in it and an over-narrow
                // filter look identical on screen and need different things
                // done about them.
                ? 'No expenses match those filters in ${monthLabel(_month)}.'
                : 'Nothing recorded in ${monthLabel(_month)}.')
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

  /// Step the month, which is what this screen has instead of pages.
  ///
  /// Both the rows and the totals are asked again: they describe one month
  /// between them, and a bottom line left over from the last one would be a
  /// figure with the wrong heading over it.
  void _step(int months) {
    setState(() => _month = addMonths(_month, months));
    _load();
  }

  /// Ask again, because the filter now matches something else.
  ///
  /// Every filter and sort change goes through here. The month is deliberately
  /// left alone: narrowing to one shop is not a reason to be thrown back to
  /// today, and a control that moved the period out from under the reader
  /// would make comparing two shops across a month impossible.
  void _refilter(VoidCallback change) {
    setState(change);
    _load();
  }

  /// Ask the core for the bottom line again, leaving the rows alone.
  ///
  /// Used after a delete, which changes what the month sums without changing
  /// which rows are on screen.
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
      // the user back to the top of the month to show them a row that is gone.
      setState(() => _rows = _rows.where((r) => r.id != txn.id).toList());
      // The totals are reloaded even though the list is not. They are the sum
      // of the whole month, so a deleted expense is still in them until they
      // are asked again — a bottom line that quietly disagrees with the rows
      // above it is worse than a moment of loading.
      await _reloadTotals();
      await _loadMenus();
      if (!mounted) return;
      // The month has nothing left in it, which is a list with something new
      // to say, and only a reload knows which message it is.
      if (_rows.isEmpty) await _load();
    } catch (error) {
      if (!mounted) return;
      setState(() => _message = 'Could not delete that expense: $error');
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final sizeClass = WindowSizeClass.of(MediaQuery.sizeOf(context).width);
    final bool table = sizeClass != WindowSizeClass.compact;
    // Where the shell floats its button over the content, mirrored from
    // `shell.dart`: everywhere except medium, which is the one width that has
    // a navigation rail to keep it in.
    final bool floatingButton = sizeClass != WindowSizeClass.medium;
    final TxnTotals? totals = _totals;
    _publishBottomLine();

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        // The same control as the Budget screen's, in the same place and with
        // the same words: the two screens are read one after the other, and a
        // month that stepped differently on each would be two mechanisms to
        // learn instead of one.
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 16, 8),
          child: Row(
            children: [
              IconButton(
                onPressed: () => _step(-1),
                icon: const Icon(Icons.chevron_left),
                tooltip: 'Previous month',
              ),
              Expanded(
                child: Text(
                  monthLabel(_month),
                  textAlign: TextAlign.center,
                  style: theme.textTheme.titleMedium,
                ),
              ),
              IconButton(
                onPressed: () => _step(1),
                icon: const Icon(Icons.chevron_right),
                tooltip: 'Next month',
              ),
            ],
          ),
        ),
        const Divider(height: 1),
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
              // So the last row has somewhere to scroll to that the button is
              // not sitting on. The bottom line below is pinned and cannot
              // scroll, which is why the button is lifted clear of it instead.
              padding: EdgeInsets.only(
                bottom: floatingButton ? _fabReserve : 0,
              ),
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
          // A live region, so an empty month or a failure is announced rather
          // than only drawn. Under the list, not above it: it is what the list
          // has to say when the rows run out, and a message above the header
          // pushes everything down every time a filter matches nothing.
          _Note(_message!),
        // Said out loud rather than left to be noticed. The totals below still
        // cover the whole month, so without this line the rows and the figure
        // under them would disagree by however many were dropped, with nothing
        // on screen to explain it.
        if (_truncated)
          _Note(
            'Showing the first ${_rows.length} of ${totals!.expenses} '
            'expenses in ${monthLabel(_month)}.',
          ),
        // Pinned under the list rather than scrolled with it: a bottom line
        // you have to reach the end of the rows to see is one nobody reads.
        // Hidden entirely when the month is empty — a row of zeroes under a
        // message saying there is nothing is the same news twice.
        //
        // Last, and under the button rather than above it: the button is
        // lifted over this row by however tall it turns out to be, so the
        // month's figure sits on the edge of the screen where a bottom line
        // belongs, instead of a strip of nothing sitting there and the total
        // floating above it.
        if (totals != null && totals.expenses > 0)
          KeyedSubtree(
            key: _bottomLineKey,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const Divider(height: 1),
                _TotalsRow(totals: totals, table: table, month: _month),
              ],
            ),
          ),
      ],
    );
  }
}

/// What the list has to say when it is not saying it in rows.
class _Note extends StatelessWidget {
  const _Note(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Semantics(
      liveRegion: true,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
        child: Text(
          text,
          style: theme.textTheme.bodyMedium
              ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
        ),
      ),
    );
  }
}

/// The bottom line: what the month comes to, and over how many expenses.
///
/// **This totals the whole month the filter matches, not the rows drawn.** The
/// figure is the core's, summed in SQL over every matching expense in the
/// month's date range, so it still covers rows the guard on the list left out.
/// Nothing here adds money: [Cents] makes that a compile error on purpose.
///
/// The month is named in the label rather than left to the control at the top
/// of the screen. A total that does not say what it covers gets read as
/// everything — the reader has no reason to suspect otherwise — and being
/// wrong about which period a figure belongs to is the whole of the harm this
/// screen can do.
class _TotalsRow extends StatelessWidget {
  const _TotalsRow({
    required this.totals,
    required this.table,
    required this.month,
  });

  final TxnTotals totals;
  final bool table;
  final String month;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final String label = totals.expenses == 1
        ? 'Total of 1 expense in ${monthLabel(month)}'
        : 'Total of all ${totals.expenses} expenses in ${monthLabel(month)}';

    final Widget money = MoneyText(
      totals.total,
      textAlign: TextAlign.right,
      style: theme.textTheme.titleMedium
          ?.copyWith(fontWeight: FontWeight.w600),
    );

    return Container(
      // Keyed so a test can ask where this row actually is on screen: the
      // complaint it exists to answer is a floating button drawn on top of it,
      // which nothing but its rectangle can pin.
      key: const Key('expense-totals'),
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
              // Flexible, because the two words do not both fit beside the
              // sort control on a 400dp phone. Given room to shrink they
              // ellipsize; denied it the row overflows, and an overflow is
              // content the reader cannot see at all.
              Flexible(
                child: FilterMenuButton(
                  label: 'Store',
                  options: stores,
                  hidden: hiddenStores,
                  onChanged: onStoresChanged,
                  dense: true,
                ),
              ),
              Flexible(
                child: FilterMenuButton(
                  label: 'Categories',
                  options: categories,
                  hidden: hiddenCategories,
                  onChanged: onCategoriesChanged,
                  dense: true,
                ),
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

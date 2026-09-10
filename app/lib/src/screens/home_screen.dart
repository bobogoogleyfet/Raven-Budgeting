import 'package:flutter/material.dart';

import '../api.dart';
import '../breakpoints.dart';
import '../fab.dart';
import '../dates.dart';
import '../money.dart';
import '../widgets/budget_ring.dart';
import '../widgets/chart_palette.dart';
import '../widgets/money_text.dart';
import '../widgets/section.dart';

/// How many recent expenses the overview shows. Three is what fits under the
/// ring on a phone, and the whole list is one tap away.
const int _recentCount = 3;

typedef BudgetLoader = Future<List<BudgetRow>> Function(String month);
typedef TotalsLoader = Future<MonthTotals> Function(String month);
typedef RecentLoader = Future<List<TxnSummary>> Function(int limit);

/// The budget at a glance, and what was spent most recently.
class HomeScreen extends StatefulWidget {
  const HomeScreen({
    super.key,
    this.onSetBudget,
    this.onOpenBudgets,
    this.loadBudget = budgetStatus,
    this.loadTotals = monthTotals,
    this.loadRecent = _recent,
    this.month,
  });

  /// Offered where a new user first notices there is no budget.
  final VoidCallback? onSetBudget;

  /// Where the full list of budgets lives.
  final VoidCallback? onOpenBudgets;

  final BudgetLoader loadBudget;
  final TotalsLoader loadTotals;
  final RecentLoader loadRecent;

  /// The month to report on, as `YYYY-MM`. Defaults to this one, read from the
  /// local clock — the core is not allowed to read it.
  final String? month;

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

Future<List<TxnSummary>> _recent(int limit) =>
    listTransactions(TxnQuery(limit: limit));

class _HomeScreenState extends State<HomeScreen> {
  late Future<(List<BudgetRow>, MonthTotals, List<TxnSummary>)> _overview;
  late String _month;

  @override
  void initState() {
    super.initState();
    _month = widget.month ?? thisMonth();
    _load();
  }

  void _load() {
    _overview = (
      widget.loadBudget(_month),
      widget.loadTotals(_month),
      widget.loadRecent(_recentCount),
    ).wait;
  }

  void _reload() => setState(_load);

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<(List<BudgetRow>, MonthTotals, List<TxnSummary>)>(
      future: _overview,
      builder: (context, snapshot) {
        if (snapshot.hasError) {
          return _Failed(message: '${snapshot.error}', onRetry: _reload);
        }
        if (!snapshot.hasData) {
          return const Center(child: CircularProgressIndicator());
        }
        final (budget, totals, recent) = snapshot.requireData;
        return _Overview(
          month: _month,
          budget: budget.where((r) => r.isBudgeted).toList(),
          unbudgeted: budget.where((r) => !r.isBudgeted).toList(),
          totals: totals,
          recent: recent,
          onSetBudget: widget.onSetBudget,
          onOpenBudgets: widget.onOpenBudgets,
        );
      },
    );
  }
}

class _Overview extends StatefulWidget {
  const _Overview({
    required this.month,
    required this.budget,
    required this.unbudgeted,
    required this.totals,
    required this.recent,
    required this.onSetBudget,
    required this.onOpenBudgets,
  });

  /// Set a budget, from the state where there are none.
  final VoidCallback? onSetBudget;

  /// Go to the screen that lists every budget there is.
  final VoidCallback? onOpenBudgets;

  final String month;
  final List<BudgetRow> budget;
  /// Categories with no budget line this month. Kept as rows rather than a
  /// count because the ones that have been *spent* on belong on the ring.
  final List<BudgetRow> unbudgeted;
  final MonthTotals totals;
  final List<TxnSummary> recent;

  @override
  State<_Overview> createState() => _OverviewState();
}

class _OverviewState extends State<_Overview> {
  /// The categories with a hue of their own, biggest budget first.
  ///
  /// Seven is the palette's ceiling; past it a hue is indistinguishable from
  /// one already in use. The tail is not dropped — it becomes one arc, so the
  /// ring still adds up to the month.
  List<BudgetRow> get _named {
    final ordered = [...widget.budget]..sort(
        (a, b) => (b.perMonth?.value ?? 0).compareTo(a.perMonth?.value ?? 0),
      );
    return ordered.take(ChartPalette.slots).toList();
  }

  List<BudgetRow> get _folded {
    final named = _named;
    return [
      for (final row in widget.budget)
        if (!named.contains(row)) row,
    ];
  }

  /// The colour a category's arc and its legend swatch share.
  Color _colourFor(BuildContext context, BudgetRow row) {
    final index = _named.indexOf(row);
    return index < 0
        ? ChartPalette.other(context)
        : ChartPalette.of(context)[index];
  }

  List<RingSegment> _segments(BuildContext context) {
    final folded = _folded;
    return [
      for (final row in _named)
        RingSegment(
          label: row.categoryName,
          budgeted: row.perMonth?.value ?? 0,
          spent: row.spentThisMonth.value,
          isOver: row.isOver,
          colour: _colourFor(context, row),
        ),
      if (_unbudgetedSpend > 0)
        RingSegment(
          label: 'Unbudgeted',
          // Sized by what was spent, because there is no budget to size it by.
          // Equal to `spent` so the arc fills completely: money spent against
          // no budget is entirely over, not partly.
          budgeted: _unbudgetedSpend,
          spent: _unbudgetedSpend,
          // Spending with nothing budgeted for it *is* the over-budget case.
          // It used to be filtered off the ring altogether, so the one kind of
          // spending nobody had planned for was the one kind the picture of
          // the month did not show.
          isOver: true,
          colour: Theme.of(context).colorScheme.error,
        ),
      if (folded.isNotEmpty)
        RingSegment(
          // One left over is not a group. Calling a single category "Other"
          // hides a name the legend is showing three lines below.
          label: folded.length == 1 ? folded.single.categoryName : 'Other',
          budgeted: folded.fold(0, (sum, r) => sum + (r.perMonth?.value ?? 0)),
          spent: folded.fold(0, (sum, r) => sum + r.spentThisMonth.value),
          isOver: folded.any((r) => r.isOver),
          colour: ChartPalette.other(context),
        ),
    ];
  }

  /// What has been spent this month on categories with no budget at all.
  int get _unbudgetedSpend =>
      widget.unbudgeted.fold(0, (sum, r) => sum + r.spentThisMonth.value);

  /// Which segment the user tapped, if any.
  int? _selected;

  /// The category behind the selected arc, or null when the selection is the
  /// folded tail or nothing at all.
  BudgetRow? get _selectedRow {
    final index = _selected;
    if (index == null || index >= _named.length) return null;
    return _named[index];
  }

  @override
  Widget build(BuildContext context) {
    final sizeClass = WindowSizeClass.of(MediaQuery.sizeOf(context).width);
    final bool wide = sizeClass != WindowSizeClass.compact;

    final budgetSection = Section(
      title: 'Budget',
      subtitle: monthLabel(widget.month),
      child: widget.budget.isEmpty
          ? Padding(
              padding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const EmptyNote('Nothing is budgeted yet.'),
                  FilledButton.tonalIcon(
                    onPressed: widget.onSetBudget,
                    icon: const Icon(Icons.add),
                    label: const Text('Set a budget'),
                  ),
                ],
              ),
            )
          : Column(
              children: [
                Padding(
                  padding: const EdgeInsets.fromLTRB(16, 8, 16, 16),
                  child: SizedBox(
                    // Big enough to read a figure inside, small enough that the
                    // legend under it is still on screen on a phone.
                    height: wide ? 280 : 240,
                    child: BudgetRing(
                      segments: _segments(context),
                      selected: _selected,
                      onSelect: (index) => setState(() => _selected = index),
                      thickness: wide ? 24 : 20,
                      centre: _Centre(
                        totals: widget.totals,
                        selected: _selectedRow,
                        folded: _selected != null && _selected! >= _named.length
                            ? _folded
                            : const [],
                      ),
                    ),
                  ),
                ),
                _Legend(
                  rows: _named,
                  folded: _folded,
                  colourOf: (row) => _colourFor(context, row),
                  otherColour: ChartPalette.other(context),
                  selected: _selected,
                  onSelect: (index) => setState(
                    () => _selected = _selected == index ? null : index,
                  ),
                  onSeeAll: widget.onOpenBudgets,
                  columns: wide ? 2 : 1,
                ),
                if (widget.unbudgeted.isNotEmpty)
                  EmptyNote(
                    '${widget.unbudgeted.length} '
                    '${widget.unbudgeted.length == 1 ? 'category has' : 'categories have'}'
                    ' no budget.',
                  ),
              ],
            ),
    );

    final recentSection = Section(
      title: 'Recent',
      subtitle: widget.recent.isEmpty ? null : 'Last $_recentCount',
      child: widget.recent.isEmpty
          ? const EmptyNote('No expenses yet.')
          : Column(
              children: [
                for (final txn in widget.recent)
                  RecentExpenseTile(txn: txn, detailed: wide),
              ],
            ),
    );

    // Space for the shell's floating button, sized to whichever button this
    // width actually gets — and nothing at medium, where it is in the rail.
    final double reserve = fabReserve(sizeClass);

    if (sizeClass == WindowSizeClass.compact ||
        sizeClass == WindowSizeClass.medium) {
      return ListView(
        padding: EdgeInsets.fromLTRB(0, 8, 0, reserve),
        children: [budgetSection, recentSection],
      );
    }
    return ListView(
      padding: EdgeInsets.fromLTRB(0, 8, 0, reserve),
      children: [
        Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Expanded(flex: 3, child: budgetSection),
            Expanded(flex: 2, child: recentSection),
          ],
        ),
      ],
    );
  }
}

/// The figure in the hole.
///
/// The one number the screen leads with, so it is the only thing at this size.
/// Proportional figures, not tabular: tabular gives every digit the width of a
/// zero, which makes a large standalone number look loose.
class _Centre extends StatelessWidget {
  const _Centre({
    required this.totals,
    required this.selected,
    this.folded = const [],
  });

  final MonthTotals totals;

  /// A tapped category, whose own figures replace the total until it is
  /// tapped again.
  final BudgetRow? selected;

  /// The categories the tapped arc stands for, when the tapped arc is the
  /// folded tail.
  final List<BudgetRow> folded;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final row = selected;
    final bool over = row?.isOver ?? false;

    final String headline = formatMoney(row?.spentThisMonth ?? totals.spent);
    final String against = formatMoney(row?.perMonth ?? totals.budgeted);

    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          row?.categoryName ??
              switch (folded.length) {
                0 => 'Spent',
                1 => folded.single.categoryName,
                final n => '$n categories',
              },
          style: theme.textTheme.labelMedium
              ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
          textAlign: TextAlign.center,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
        ),
        const SizedBox(height: 2),
        FittedBox(
          fit: BoxFit.scaleDown,
          child: Text(
            headline,
            style: theme.textTheme.headlineMedium?.copyWith(
              fontWeight: FontWeight.w600,
              color: over ? theme.colorScheme.error : theme.colorScheme.onSurface,
            ),
            maxLines: 1,
          ),
        ),
        FittedBox(
          fit: BoxFit.scaleDown,
          child: Text(
            'of $against',
            style: theme.textTheme.bodyMedium
                ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
            maxLines: 1,
          ),
        ),
      ],
    );
  }
}

/// Which arc is which.
///
/// The ring is one colour, so this is where identity lives. Over budget is
/// named in words and carries an icon as well as the colour — a state told by
/// hue alone is a state some readers are not told at all.
/// Which arc is which.
///
/// It names what the ring draws and nothing else. Every budgeted category
/// listed here would make the overview a worse copy of the Budget screen —
/// which lists them all, in groups, with the figures behind them — and would
/// turn a summary into a page you scroll. So the tail is one row that says how
/// many it stands for, and the way to see them all is to go where they are.
class _Legend extends StatelessWidget {
  const _Legend({
    required this.rows,
    required this.folded,
    required this.colourOf,
    required this.otherColour,
    required this.selected,
    required this.onSelect,
    required this.onSeeAll,
    required this.columns,
  });

  final List<BudgetRow> rows;
  final List<BudgetRow> folded;
  final Color Function(BudgetRow row) colourOf;
  final Color otherColour;
  final int? selected;
  final void Function(int index) onSelect;
  final VoidCallback? onSeeAll;

  /// Two columns where there is width for them: a legend of seven rows is
  /// taller than the ring it explains, and the pair read as one block.
  final int columns;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    final entries = <Widget>[
      for (int i = 0; i < rows.length; i++)
        _LegendRow(
          label: rows[i].categoryName,
          amount: rows[i].isOver ? rows[i].overspentBy! : rows[i].remaining!,
          over: rows[i].isOver,
          note: _noteFor(rows[i]),
          colour: colourOf(rows[i]),
          selected: selected == i,
          onTap: () => onSelect(i),
          theme: theme,
        ),
      if (folded.isNotEmpty)
        _LegendRow(
          label: folded.length == 1
              ? folded.single.categoryName
              : '${folded.length} more categories',
          // One category still has a figure worth showing. Several do not: a
          // sum of what is left across them is a number nobody asked for, and
          // they are all on the Budget screen.
          amount: folded.length == 1
              ? (folded.single.isOver
                  ? folded.single.overspentBy
                  : folded.single.remaining)
              : null,
          over: folded.any((r) => r.isOver),
          note: null,
          colour: otherColour,
          selected: selected == rows.length,
          onTap: () => onSelect(rows.length),
          theme: theme,
        ),
    ];

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (columns == 1)
          ...entries
        else
          for (int i = 0; i < entries.length; i += 2)
            Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Expanded(child: entries[i]),
                Expanded(
                  child: i + 1 < entries.length
                      ? entries[i + 1]
                      : const SizedBox.shrink(),
                ),
              ],
            ),
        if (onSeeAll != null)
          Align(
            alignment: Alignment.centerLeft,
            child: Padding(
              padding: const EdgeInsets.fromLTRB(8, 4, 8, 0),
              child: TextButton(
                onPressed: onSeeAll,
                child: const Text('See all budgets'),
              ),
            ),
          ),
      ],
    );
  }

  /// The small print a wider window has room for.
  String? _noteFor(BudgetRow row) {
    if (columns < 2) return null;
    final parts = [
      if (row.isMultiMonth) '${row.monthsRemaining} mo cycle',
      // Funding is a different question from over-budget and keeps different
      // words and no colour: rent still unpaid is behind on funding with
      // nothing spent at all, and a sinking fund is behind by what was never
      // set aside.
      if (row.funding == 'behind') 'behind ${formatMoney(row.fundingBy)}',
      if (row.funding == 'ahead') 'ahead ${formatMoney(row.fundingBy)}',
    ];
    return parts.isEmpty ? null : parts.join(' · ');
  }
}

class _LegendRow extends StatelessWidget {
  const _LegendRow({
    required this.label,
    required this.amount,
    required this.over,
    required this.note,
    required this.colour,
    required this.selected,
    required this.onTap,
    required this.theme,
  });

  final String label;

  /// Null on the folded row: a sum of what is left across several categories
  /// is a number nobody asked for, and the ones it hides are on the Budget
  /// screen.
  final Cents? amount;

  final bool over;
  final String? note;
  final Color colour;
  final bool selected;
  final VoidCallback onTap;
  final ThemeData theme;

  @override
  Widget build(BuildContext context) {
    final Color amountColour =
        over ? theme.colorScheme.error : theme.colorScheme.onSurface;

    return InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(8),
      child: Container(
        decoration: BoxDecoration(
          color: selected
              ? theme.colorScheme.primaryContainer.withValues(alpha: 0.45)
              : null,
          borderRadius: BorderRadius.circular(8),
        ),
        margin: const EdgeInsets.symmetric(horizontal: 8, vertical: 1),
        padding: const EdgeInsets.fromLTRB(8, 8, 8, 8),
        child: Row(
          children: [
            // The swatch is what makes this a legend rather than a list: it is
            // the same colour the arc is drawn in, including the red an
            // over-budget category takes.
            Container(
              width: 12,
              height: 12,
              decoration: BoxDecoration(
                color: over ? theme.colorScheme.error : colour,
                borderRadius: BorderRadius.circular(3),
              ),
            ),
            const SizedBox(width: 10),
            if (over) ...[
              Icon(Icons.error_outline, size: 16, color: theme.colorScheme.error),
              const SizedBox(width: 4),
            ],
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    label,
                    style: theme.textTheme.bodyLarge,
                    overflow: TextOverflow.ellipsis,
                  ),
                  if (note != null)
                    Text(
                      note!,
                      style: theme.textTheme.bodySmall
                          ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
                      overflow: TextOverflow.ellipsis,
                    ),
                ],
              ),
            ),
            if (amount != null) ...[
              const SizedBox(width: 8),
              MoneyText(
                amount!,
                textAlign: TextAlign.right,
                style: theme.textTheme.titleMedium?.copyWith(color: amountColour),
              ),
              const SizedBox(width: 4),
              SizedBox(
                width: 30,
                child: Text(
                  over ? 'over' : 'left',
                  style: theme.textTheme.labelMedium?.copyWith(color: amountColour),
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

/// One of the most recent expenses.
class RecentExpenseTile extends StatelessWidget {
  const RecentExpenseTile({
    super.key,
    required this.txn,
    required this.detailed,
  });

  final TxnSummary txn;
  final bool detailed;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return ListTile(
      contentPadding: const EdgeInsets.symmetric(horizontal: 16),
      title: Text(txn.merchantName ?? 'No store'),
      subtitle: Text(
        detailed && txn.categories.isNotEmpty
            ? '${txn.txnDate} · ${txn.categories.join(', ')}'
            : txn.txnDate,
        overflow: TextOverflow.ellipsis,
      ),
      trailing: MoneyText(txn.total, style: theme.textTheme.titleMedium),
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
            Text(message, textAlign: TextAlign.center),
            const SizedBox(height: 16),
            FilledButton.tonal(onPressed: onRetry, child: const Text('Retry')),
          ],
        ),
      ),
    );
  }
}

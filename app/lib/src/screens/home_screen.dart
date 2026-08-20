import 'package:flutter/material.dart';

import '../api.dart';
import '../breakpoints.dart';
import '../dates.dart';
import '../money.dart';
import '../widgets/budget_ring.dart';
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
    this.loadBudget = budgetStatus,
    this.loadTotals = monthTotals,
    this.loadRecent = _recent,
    this.month,
  });

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
          unbudgeted: budget.where((r) => !r.isBudgeted).length,
          totals: totals,
          recent: recent,
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
  });

  final String month;
  final List<BudgetRow> budget;
  final int unbudgeted;
  final MonthTotals totals;
  final List<TxnSummary> recent;

  @override
  State<_Overview> createState() => _OverviewState();
}

class _OverviewState extends State<_Overview> {
  /// Which segment the user tapped, if any. Tapping is how a segment says
  /// which category it is — the ring itself is one colour, because colour here
  /// means state and not identity.
  int? _selected;

  @override
  Widget build(BuildContext context) {
    final sizeClass = WindowSizeClass.of(MediaQuery.sizeOf(context).width);
    final bool wide = sizeClass != WindowSizeClass.compact;

    final budgetSection = Section(
      title: 'Budget',
      subtitle: monthLabel(widget.month),
      child: widget.budget.isEmpty
          ? const EmptyNote(
              'Nothing is budgeted yet.\n'
              'Set one with: pacc budget set --category NAME --amount N',
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
                      segments: [
                        for (final row in widget.budget)
                          RingSegment(
                            label: row.categoryName,
                            budgeted: row.perMonth?.value ?? 0,
                            spent: row.spentThisMonth.value,
                            isOver: row.isOver,
                          ),
                      ],
                      selected: _selected,
                      onSelect: (index) => setState(() => _selected = index),
                      thickness: wide ? 24 : 20,
                      centre: _Centre(
                        totals: widget.totals,
                        selected: _selected == null
                            ? null
                            : widget.budget[_selected!],
                      ),
                    ),
                  ),
                ),
                _Legend(
                  rows: widget.budget,
                  selected: _selected,
                  onSelect: (index) => setState(
                    () => _selected = _selected == index ? null : index,
                  ),
                  wide: wide,
                ),
                if (widget.unbudgeted > 0)
                  EmptyNote(
                    '${widget.unbudgeted} '
                    '${widget.unbudgeted == 1 ? 'category has' : 'categories have'}'
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

    if (sizeClass == WindowSizeClass.compact ||
        sizeClass == WindowSizeClass.medium) {
      return ListView(
        padding: const EdgeInsets.fromLTRB(0, 8, 0, 88),
        children: [budgetSection, recentSection],
      );
    }
    return ListView(
      padding: const EdgeInsets.fromLTRB(0, 8, 0, 88),
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
  const _Centre({required this.totals, required this.selected});

  final MonthTotals totals;

  /// A tapped category, whose own figures replace the total until it is
  /// tapped again.
  final BudgetRow? selected;

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
          row?.categoryName ?? 'Spent',
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
class _Legend extends StatelessWidget {
  const _Legend({
    required this.rows,
    required this.selected,
    required this.onSelect,
    required this.wide,
  });

  final List<BudgetRow> rows;
  final int? selected;
  final void Function(int index) onSelect;
  final bool wide;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Column(
      children: [
        for (int i = 0; i < rows.length; i++)
          _LegendRow(
            row: rows[i],
            selected: selected == i,
            onTap: () => onSelect(i),
            wide: wide,
            theme: theme,
          ),
      ],
    );
  }
}

class _LegendRow extends StatelessWidget {
  const _LegendRow({
    required this.row,
    required this.selected,
    required this.onTap,
    required this.wide,
    required this.theme,
  });

  final BudgetRow row;
  final bool selected;
  final VoidCallback onTap;
  final bool wide;
  final ThemeData theme;

  @override
  Widget build(BuildContext context) {
    final bool over = row.isOver;
    final Color amountColour =
        over ? theme.colorScheme.error : theme.colorScheme.onSurface;

    return InkWell(
      onTap: onTap,
      child: Container(
        color: selected
            ? theme.colorScheme.primaryContainer.withValues(alpha: 0.35)
            : null,
        padding: const EdgeInsets.fromLTRB(16, 10, 16, 10),
        child: Row(
          children: [
            if (over) ...[
              Icon(
                Icons.error_outline,
                size: 16,
                color: theme.colorScheme.error,
              ),
              const SizedBox(width: 6),
            ],
            Expanded(
              child: Text(
                row.categoryName,
                style: theme.textTheme.bodyLarge,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            if (wide) ...[
              // Funding is a different question from over-budget and keeps
              // different words and no colour: an envelope spent exactly to
              // plan is behind on funding and perfectly fine on spending, and
              // flagging that would train the reader to ignore the screen.
              Text(
                [
                  if (row.isMultiMonth) '${row.monthsRemaining} mo cycle',
                  if (row.funding == 'behind')
                    'behind ${formatMoney(row.fundingBy)}',
                  if (row.funding == 'ahead')
                    'ahead ${formatMoney(row.fundingBy)}',
                ].join(' · '),
                style: theme.textTheme.bodySmall
                    ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
              ),
              const SizedBox(width: 12),
            ],
            // Fixed widths so the amounts line up down the column: a trailing
            // word of a different length otherwise pushes the number about.
            SizedBox(
              width: 104,
              // "Left" and "over" are one fact from either side, so only one
              // is ever on screen.
              child: MoneyText(
                over ? row.overspentBy! : row.remaining!,
                textAlign: TextAlign.right,
                style: theme.textTheme.titleMedium?.copyWith(color: amountColour),
              ),
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

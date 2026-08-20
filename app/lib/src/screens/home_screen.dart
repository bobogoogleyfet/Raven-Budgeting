import 'package:flutter/material.dart';

import '../api.dart';
import '../breakpoints.dart';
import '../dates.dart';
import '../money.dart';
import '../widgets/money_text.dart';
import '../widgets/section.dart';

/// How many recent expenses the overview shows. Three is what fits above the
/// fold on a phone next to the budget, and the whole list is one tap away.
const int _recentCount = 3;

typedef BudgetLoader = Future<List<BudgetRow>> Function(String month);
typedef RecentLoader = Future<List<TxnSummary>> Function(int limit);

/// The budget at a glance, and what was spent most recently.
///
/// Two questions, in the order they get asked: is there money left, and what
/// went out. Everything else is a tap away.
class HomeScreen extends StatefulWidget {
  const HomeScreen({
    super.key,
    this.loadBudget = budgetStatus,
    this.loadRecent = _recent,
    this.month,
  });

  final BudgetLoader loadBudget;
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
  late Future<(List<BudgetRow>, List<TxnSummary>)> _overview;
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
      widget.loadRecent(_recentCount),
    ).wait;
  }

  void _reload() {
    setState(_load);
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<(List<BudgetRow>, List<TxnSummary>)>(
      future: _overview,
      builder: (context, snapshot) {
        if (snapshot.hasError) {
          return _Failed(message: '${snapshot.error}', onRetry: _reload);
        }
        if (!snapshot.hasData) {
          return const Center(child: CircularProgressIndicator());
        }
        final (budget, recent) = snapshot.requireData;
        return _Overview(
          month: _month,
          budget: budget.where((r) => r.isBudgeted).toList(),
          unbudgeted: budget.where((r) => !r.isBudgeted).length,
          recent: recent,
        );
      },
    );
  }
}

class _Overview extends StatelessWidget {
  const _Overview({
    required this.month,
    required this.budget,
    required this.unbudgeted,
    required this.recent,
  });

  final String month;
  final List<BudgetRow> budget;
  final int unbudgeted;
  final List<TxnSummary> recent;

  @override
  Widget build(BuildContext context) {
    final sizeClass = WindowSizeClass.of(MediaQuery.sizeOf(context).width);
    final detailed = sizeClass != WindowSizeClass.compact;

    final budgetSection = Section(
      title: 'Budget',
      subtitle: monthLabel(month),
      child: budget.isEmpty
          ? const EmptyNote(
              'Nothing is budgeted yet.\nSet one with: pacc budget set --category NAME --amount N',
            )
          : Column(
              children: [
                for (final row in budget)
                  BudgetRowTile(row: row, detailed: detailed),
                if (unbudgeted > 0)
                  EmptyNote(
                    '$unbudgeted ${unbudgeted == 1 ? 'category has' : 'categories have'} no budget.',
                  ),
              ],
            ),
    );

    final recentSection = Section(
      title: 'Recent',
      subtitle: recent.isEmpty ? null : 'Last $_recentCount',
      child: recent.isEmpty
          ? const EmptyNote('No expenses yet.')
          : Column(
              children: [
                for (final txn in recent)
                  RecentExpenseTile(txn: txn, detailed: detailed),
              ],
            ),
    );

    // Side by side once there is width for it, and stacked on a phone with the
    // budget first: "is there money left" is the question the app is opened to
    // answer, and the recent expenses are the evidence under it.
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

/// One category's budget.
///
/// On a phone: the name, and what is left or how far over. Where there is
/// width, the figures behind that — spent, of how much, and how the funding is
/// doing — because the number that matters is the same either way and the
/// working is what a wider window has room for.
class BudgetRowTile extends StatelessWidget {
  const BudgetRowTile({super.key, required this.row, required this.detailed});

  final BudgetRow row;
  final bool detailed;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final over = row.isOver;
    final Color amountColour =
        over ? theme.colorScheme.error : theme.colorScheme.onSurface;

    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 10, 16, 10),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Expanded(
                child: Text(
                  row.categoryName,
                  style: theme.textTheme.bodyLarge,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              const SizedBox(width: 12),
              // Fixed widths so the amounts line up down the column. Tabular
              // figures only align digits with each other; a trailing word of
              // a different length still pushes the number about, and a column
              // of amounts that does not line up is one you read twice.
              SizedBox(
                width: 104,
                // "Left" and "over" are the same fact from either side, so
                // only one of them is ever on screen.
                child: MoneyText(
                  over ? row.overspentBy! : row.remaining!,
                  textAlign: TextAlign.right,
                  style:
                      theme.textTheme.titleMedium?.copyWith(color: amountColour),
                ),
              ),
              const SizedBox(width: 4),
              SizedBox(
                width: 30,
                child: Text(
                  over ? 'over' : 'left',
                  style:
                      theme.textTheme.labelMedium?.copyWith(color: amountColour),
                ),
              ),
            ],
          ),
          const SizedBox(height: 6),
          _Bar(row: row),
          if (detailed) ...[
            const SizedBox(height: 6),
            _Figures(row: row),
          ],
        ],
      ),
    );
  }
}

/// How much of the cycle's budget is gone.
class _Bar extends StatelessWidget {
  const _Bar({required this.row});

  final BudgetRow row;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final int budgeted = row.budgeted?.value ?? 0;
    final int spent = row.spentCycle?.value ?? 0;
    // A ratio for pixels, not a monetary figure: it produces a fraction of a
    // bar's width, is clamped, and is never sent anywhere. Every amount on
    // screen still comes from the core.
    final double fraction =
        budgeted <= 0 ? 0 : (spent / budgeted).clamp(0.0, 1.0);

    return ClipRRect(
      borderRadius: BorderRadius.circular(4),
      child: LinearProgressIndicator(
        value: fraction,
        minHeight: 6,
        // Not `surfaceContainerHighest`: that is exactly what a filled card is
        // painted with, so the track vanished into it and a category with
        // nothing spent looked like it had no bar at all.
        backgroundColor: theme.colorScheme.outlineVariant,
        color: row.isOver ? theme.colorScheme.error : theme.colorScheme.primary,
      ),
    );
  }
}

class _Figures extends StatelessWidget {
  const _Figures({required this.row});

  final BudgetRow row;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final style = theme.textTheme.bodySmall
        ?.copyWith(color: theme.colorScheme.onSurfaceVariant);
    final parts = <String>[
      '${formatMoney(row.spentCycle!)} of ${formatMoney(row.budgeted!)}',
      // A cycle longer than a month has to say so, or "left" reads as left of
      // this month rather than left of the year's bill.
      if (row.isMultiMonth) '${row.monthsRemaining} months left in cycle',
      // Funding is a different question from over-budget and gets different
      // words: an envelope spent exactly to plan is behind on funding and
      // perfectly fine on spending.
      if (row.funding == 'behind') 'behind ${formatMoney(row.fundingBy)}',
      if (row.funding == 'ahead') 'ahead ${formatMoney(row.fundingBy)}',
    ];
    return Text(parts.join(' · '), style: style);
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

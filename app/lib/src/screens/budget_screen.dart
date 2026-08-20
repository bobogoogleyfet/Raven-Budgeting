import 'package:flutter/material.dart';

import '../api.dart';
import '../breakpoints.dart';
import '../dates.dart';
import '../money.dart';
import '../widgets/money_text.dart';

typedef BudgetLoader = Future<List<BudgetRow>> Function(String month);

/// The budget screen.
///
/// # Funding and overspending are different columns on purpose
///
/// They answer different questions and disagree in normal use. A $400/month
/// grocery envelope with $400 spent is *behind on funding* — the schedule says
/// that money should be banked and it is not — while being exactly on budget
/// on spending. Rendering "behind" as "over budget" would flag every well-run
/// category and train the user to ignore the screen.
///
/// So funding is its own column, and the over-budget highlight keys strictly on
/// the core's `overspentBy`, which it sets only when spending has actually
/// passed the cycle's budget.
class BudgetScreen extends StatefulWidget {
  const BudgetScreen({super.key, this.load = budgetStatus, this.month});

  final BudgetLoader load;
  final String? month;

  @override
  State<BudgetScreen> createState() => _BudgetScreenState();
}

class _BudgetScreenState extends State<BudgetScreen> {
  late String _month = widget.month ?? thisMonth();
  late Future<List<BudgetRow>> _rows;

  @override
  void initState() {
    super.initState();
    _rows = widget.load(_month);
  }

  void _step(int months) {
    final parts = _month.split('-');
    final DateTime moved = DateTime(
      int.parse(parts[0]),
      int.parse(parts[1]) + months,
    );
    setState(() {
      _month = thisMonth(moved);
      _rows = widget.load(_month);
    });
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final bool wide =
        WindowSizeClass.of(MediaQuery.sizeOf(context).width) !=
            WindowSizeClass.compact;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
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
        Expanded(
          child: FutureBuilder<List<BudgetRow>>(
            future: _rows,
            builder: (context, snapshot) {
              if (snapshot.hasError) {
                return _Message('Could not load budgets: ${snapshot.error}');
              }
              if (!snapshot.hasData) {
                return const Center(child: CircularProgressIndicator());
              }
              final all = snapshot.requireData;
              final budgeted = all.where((r) => r.isBudgeted).toList();
              final int without = all.length - budgeted.length;

              if (budgeted.isEmpty) {
                return _Message(
                  'No budgets set for this month.\n'
                  'Set one with: pacc budget set --category NAME --amount N',
                );
              }

              return ListView(
                padding: const EdgeInsets.only(bottom: 88),
                children: [
                  for (final row in budgeted) _BudgetLine(row: row, wide: wide),
                  if (without > 0)
                    _Message(
                      '$without ${without == 1 ? 'category has' : 'categories have'}'
                      ' no budget — set one with `pacc budget set`.',
                    ),
                  const _Footnote(),
                ],
              );
            },
          ),
        ),
      ],
    );
  }
}

class _BudgetLine extends StatelessWidget {
  const _BudgetLine({required this.row, required this.wide});

  final BudgetRow row;
  final bool wide;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final bool over = row.isOver;
    // Only over-budget gets the wash. Behind-on-funding is a different tone in
    // a different column, because it is a different question.
    final Color? background =
        over ? theme.colorScheme.errorContainer.withValues(alpha: 0.35) : null;

    final String funding = switch (row.funding) {
      'none' => '—',
      'on track' => 'on track',
      final other => '$other ${formatMoney(row.fundingBy)}',
    };

    return Container(
      color: background,
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Expanded(
                child: Text(
                  '${row.groupName} / ${row.categoryName}',
                  style: theme.textTheme.bodyLarge,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              if (wide) ...[
                _Cell(label: 'Per month', amount: row.perMonth),
                _Cell(label: 'Funded', amount: row.funded),
              ],
              _Cell(label: 'Spent (cycle)', amount: row.spentCycle),
              _Cell(label: 'Left', amount: row.remaining),
              SizedBox(
                width: 120,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.end,
                  children: [
                    Text('Over',
                        style: theme.textTheme.labelSmall?.copyWith(
                            color: theme.colorScheme.onSurfaceVariant)),
                    // Positive and unsigned, and an em dash rather than $0.00:
                    // zero over budget is not a figure, it is the absence of
                    // one.
                    over
                        ? MoneyText(
                            row.overspentBy!,
                            textAlign: TextAlign.right,
                            style: theme.textTheme.bodyMedium
                                ?.copyWith(color: theme.colorScheme.error),
                          )
                        : Text('—', style: theme.textTheme.bodyMedium),
                  ],
                ),
              ),
            ],
          ),
          Padding(
            padding: const EdgeInsets.only(top: 2),
            child: Text(
              funding,
              style: theme.textTheme.bodySmall?.copyWith(
                // Behind gets a warning tone, never the error tone: an envelope
                // spent exactly to plan is behind on funding and perfectly fine
                // on spending.
                color: row.funding == 'behind'
                    ? theme.colorScheme.tertiary
                    : theme.colorScheme.onSurfaceVariant,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _Cell extends StatelessWidget {
  const _Cell({required this.label, required this.amount});

  final String label;
  final Cents? amount;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return SizedBox(
      width: 120,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.end,
        children: [
          Text(label,
              style: theme.textTheme.labelSmall
                  ?.copyWith(color: theme.colorScheme.onSurfaceVariant)),
          amount == null
              ? Text('—', style: theme.textTheme.bodyMedium)
              : MoneyText(
                  amount!,
                  textAlign: TextAlign.right,
                  style: theme.textTheme.bodyMedium,
                ),
        ],
      ),
    );
  }
}

class _Footnote extends StatelessWidget {
  const _Footnote();

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
      child: Text(
        'Funding asks whether the money will be there when the bill lands; '
        'Over asks whether more has been spent than this cycle’s budget. '
        'They disagree by design — an envelope spent exactly to plan is behind '
        'on funding and fine on spending.',
        style: theme.textTheme.bodySmall
            ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
      ),
    );
  }
}

class _Message extends StatelessWidget {
  const _Message(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.all(16),
      child: Semantics(
        liveRegion: true,
        child: Text(
          text,
          style: theme.textTheme.bodyMedium
              ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
        ),
      ),
    );
  }
}

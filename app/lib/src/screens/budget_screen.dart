import 'package:flutter/material.dart';

import '../api.dart';
import '../breakpoints.dart';
import '../dates.dart';
import '../money.dart';
import '../widgets/cents_field.dart';
import '../widgets/money_text.dart';
import 'set_budget_dialog.dart';

typedef BudgetLoader = Future<List<BudgetRow>> Function(String month);
typedef CategoryLoader = Future<List<CategoryDto>> Function();

/// The budget screen.
///
/// # Funding and overspending are different columns on purpose
///
/// They answer different questions and disagree in normal use. A $900 rent
/// line nobody has paid yet is *behind on funding* — the bill is still due —
/// while having spent nothing at all, and a yearly premium reads behind by
/// whatever was never actually set aside rather than by what the plan said.
/// Rendering "behind" as "over budget" would flag every bill still waiting to
/// be paid and train the user to ignore the screen.
///
/// So funding is its own column, and the over-budget highlight keys strictly on
/// the core's `overspentBy`, which it sets only when spending has actually
/// passed the cycle's budget.
class BudgetScreen extends StatefulWidget {
  const BudgetScreen({
    super.key,
    this.load = budgetStatus,
    this.loadCategories = listCategories,
    this.month,
    this.record = recordContribution,
  });

  final BudgetLoader load;
  final CategoryLoader loadCategories;
  final String? month;
  final Future<void> Function({
    required int categoryId,
    required String month,
    required Cents amount,
  }) record;

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

  String? _note;

  /// Record what was actually set aside for a sinking fund this month.
  ///
  /// Prefilled with the month's target and editable before it lands, because
  /// saving exactly the planned amount is the normal case and a tight month is
  /// the one worth being able to say out loud. Recording nothing at all is
  /// already what an untouched month means, so there is no "skip" button.
  Future<void> _recordSaved(BudgetRow row) async {
    final Cents? amount = await showDialog<Cents>(
      context: context,
      builder: (context) => _SavedDialog(row: row, month: _month),
    );
    if (amount == null || !mounted) return;
    try {
      await widget.record(
        categoryId: row.categoryId,
        month: _month,
        amount: amount,
      );
    } catch (error) {
      if (mounted) setState(() => _note = 'Could not record it: $error');
      return;
    }
    if (!mounted) return;
    setState(() {
      _note = '${row.categoryName}: ${formatMoney(amount)} set aside.';
      _rows = widget.load(_month);
    });
  }

  /// Open the budget editor, and say what it did.
  ///
  /// Reachable from the screen that shows budgets and from the overview's
  /// empty state, because those are the two places someone notices a budget is
  /// missing. Before this, the only way to set one was `pacc budget set`.
  Future<void> _setBudget([BudgetRow? row]) async {
    final List<CategoryDto> categories;
    try {
      categories = await widget.loadCategories();
    } catch (error) {
      if (mounted) setState(() => _note = 'Could not load categories: $error');
      return;
    }
    if (!mounted) return;

    final result = await showDialog<Superseded>(
      context: context,
      builder: (context) => SetBudgetDialog(
        categories: categories,
        month: _month,
        initialCategoryId: row?.categoryId,
      ),
    );
    if (result == null || !mounted) return;

    final name =
        row?.categoryName ??
        categories
            .firstWhere(
              (c) => c.id == row?.categoryId,
              orElse: () => categories.first,
            )
            .name;
    setState(() {
      _note = budgetOutcome(result, row?.categoryName ?? name);
      _rows = widget.load(_month);
    });
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
      _note = null;
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
              const SizedBox(width: 8),
              FilledButton.tonalIcon(
                onPressed: () => _setBudget(),
                icon: const Icon(Icons.add),
                label: const Text('Set a budget'),
              ),
            ],
          ),
        ),
        if (_note != null)
          Semantics(
            liveRegion: true,
            child: Padding(
              padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
              child: Text(_note!, style: theme.textTheme.bodyMedium),
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
                return _Message('No budgets set for this month.');
              }

              return ListView(
                padding: const EdgeInsets.only(bottom: 88),
                children: [
                  for (final row in budgeted)
                    _BudgetLine(
                      row: row,
                      wide: wide,
                      onEdit: () => _setBudget(row),
                      onRecordSaved: () => _recordSaved(row),
                    ),
                  if (without > 0)
                    _Message(
                      '$without ${without == 1 ? 'category has' : 'categories have'}'
                      ' no budget.',
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
  const _BudgetLine({
    required this.row,
    required this.wide,
    required this.onEdit,
    required this.onRecordSaved,
  });

  final BudgetRow row;
  final bool wide;
  final VoidCallback onEdit;
  final VoidCallback onRecordSaved;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final bool over = row.isOver;
    // Only over-budget gets the wash. Behind-on-funding is a different tone in
    // a different column, because it is a different question.
    final Color? background = over
        ? theme.colorScheme.errorContainer.withValues(alpha: 0.35)
        : null;

    // "Behind" only means something where the money is owed. Rent budgeted and
    // unpaid is $900 behind; groceries budgeted and unspent is $250 *left*, and
    // calling that being behind reads as a telling-off for the good outcome.
    // The core decides which kind of group this is, from the group, rather than
    // each screen guessing from its name.
    final String funding = switch (row.funding) {
      'none' => '—',
      'on track' when row.groupIsObligation => 'on track',
      // For a spending envelope there is nothing to be on track *against*: the
      // useful sentence is simply what has gone out of it.
      'on track' => 'spent ${formatMoney(row.spentThisMonth)}',
      'behind' when !row.groupIsObligation =>
        'spent ${formatMoney(row.spentThisMonth)}, ${formatMoney(row.fundingBy)} left',
      'ahead' when !row.groupIsObligation => 'spent ${formatMoney(row.spentThisMonth)}',
      final other => '$other ${formatMoney(row.fundingBy)}',
    };

    return InkWell(
      onTap: onEdit,
      child: Container(
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
                      Text(
                        'Over',
                        style: theme.textTheme.labelSmall?.copyWith(
                          color: theme.colorScheme.onSurfaceVariant,
                        ),
                      ),
                      // Positive and unsigned, and an em dash rather than $0.00:
                      // zero over budget is not a figure, it is the absence of
                      // one.
                      over
                          ? MoneyText(
                              row.overspentBy!,
                              textAlign: TextAlign.right,
                              style: theme.textTheme.bodyMedium?.copyWith(
                                color: theme.colorScheme.error,
                              ),
                            )
                          : Text('—', style: theme.textTheme.bodyMedium),
                    ],
                  ),
                ),
              ],
            ),
            if (row.isSinkingFund)
              Align(
                alignment: Alignment.centerLeft,
                child: Padding(
                  padding: const EdgeInsets.only(top: 4),
                  // A sinking fund is funded by what is recorded here and by
                  // nothing else, so the row that reports it behind is also the
                  // row that can fix it. Anywhere else and the two would be a
                  // navigation apart.
                  child: row.contributedThisMonth == null
                      ? TextButton.icon(
                          onPressed: onRecordSaved,
                          icon: const Icon(Icons.savings_outlined, size: 18),
                          label: Text(
                            'Set aside ${formatMoney(row.perMonth ?? zero)}',
                          ),
                        )
                      : TextButton.icon(
                          onPressed: onRecordSaved,
                          icon: const Icon(Icons.check_circle_outline, size: 18),
                          label: Text(
                            '${formatMoney(row.contributedThisMonth!)} set aside',
                          ),
                        ),
                ),
              ),
            Padding(
              padding: const EdgeInsets.only(top: 2),
              child: Text(
                funding,
                style: theme.textTheme.bodySmall?.copyWith(
                  // Behind gets a warning tone, never the error tone: a bill
                  // still due is not the same news as money already overspent.
                  // The warning tone belongs to a bill that has not been paid.
                  // An envelope with money still in it is not a warning, so it
                  // reads in the ordinary colour however far "behind" the
                  // funding arithmetic says it is.
                  color: row.funding == 'behind' && row.groupIsObligation
                      ? theme.colorScheme.tertiary
                      : theme.colorScheme.onSurfaceVariant,
                ),
              ),
            ),
          ],
        ),
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
          Text(
            label,
            style: theme.textTheme.labelSmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
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
        'Bills and savings report whether the money is there yet — an unpaid '
        'bill is behind by what it still owes. Spending categories just report '
        'what has gone out, because an envelope with money left in it is not '
        'behind on anything.',
        style: theme.textTheme.bodySmall?.copyWith(
          color: theme.colorScheme.onSurfaceVariant,
        ),
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
          style: theme.textTheme.bodyMedium?.copyWith(
            color: theme.colorScheme.onSurfaceVariant,
          ),
        ),
      ),
    );
  }
}


/// Asks what was actually set aside, prefilled with what was planned.
class _SavedDialog extends StatefulWidget {
  const _SavedDialog({required this.row, required this.month});

  final BudgetRow row;
  final String month;

  @override
  State<_SavedDialog> createState() => _SavedDialogState();
}

class _SavedDialogState extends State<_SavedDialog> {
  late final TextEditingController _amount = TextEditingController(
    // The plan, as the opening offer. Saving exactly the target is the normal
    // case, and typing a figure the app already knows is the decision this
    // form should not be asking for.
    text: formatCents(
      widget.row.contributedThisMonth ?? widget.row.perMonth ?? zero,
    ),
  );

  @override
  void dispose() {
    _amount.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final Cents? parsed = amountValue(_amount.text);
    return AlertDialog(
      title: Text('Set aside for ${widget.row.categoryName}'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            'Money moved into this fund in ${widget.month}. It is not spending: '
            'it will not count against the budget now, and it will not be '
            'counted again when the bill is paid.',
            style: Theme.of(context).textTheme.bodySmall,
          ),
          const SizedBox(height: 12),
          CentsField(
            controller: _amount,
            autofocus: true,
            onChanged: (_) => setState(() {}),
          ),
        ],
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed:
              parsed == null ? null : () => Navigator.of(context).pop(parsed),
          child: const Text('Record'),
        ),
      ],
    );
  }
}

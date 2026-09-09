import 'package:flutter/material.dart';

import '../api.dart';
import '../dates.dart';
import '../money.dart';
import '../widgets/cents_field.dart';
import '../widgets/money_text.dart';
import 'set_budget_dialog.dart';

/// One money column of the table, in device-independent pixels.
///
/// Wide enough for six figures and a sign without the amounts touching, and
/// the same number in the rows and in the bottom line so a total sits under
/// the column it adds up.
const double _columnWidth = 120;

/// The narrowest a category name may be squeezed to and still be a name.
///
/// "Non-Monthly / Car Insurance" is what has to survive; below this it is read
/// as "Non-M…" and the row stops saying which budget it is.
const double _nameFloor = 180;

/// How much room the table needs before it is worth offering.
///
/// Derived rather than chosen: five columns, the row's own padding, and a name
/// still legible beside them. Which navigation component is on screen is a
/// different question and the window size class answers that one — but the
/// rail and the standard drawer take their width out of *this* screen, so a
/// 1000dp window can hand the table 640dp. Asking the space itself is the only
/// way the answer is about the space.
const double _tableWidth = 5 * _columnWidth + 32 + _nameFloor;

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
    this.loadTotals = budgetTotals,
  });

  final BudgetLoader load;
  final CategoryLoader loadCategories;
  final String? month;
  final Future<void> Function({
    required int categoryId,
    required String month,
    required Cents amount,
  }) record;
  final Future<BudgetTotals> Function(String month) loadTotals;

  @override
  State<BudgetScreen> createState() => _BudgetScreenState();
}

class _BudgetScreenState extends State<BudgetScreen> {
  late String _month = widget.month ?? thisMonth();
  late Future<List<BudgetRow>> _rows;
  /// Asked for alongside the rows, and summed in the core from the same set —
  /// a footer that re-reads the database can sit under a table it disagrees
  /// with, with nothing on screen to say so.
  late Future<BudgetTotals> _totals;

  @override
  void initState() {
    super.initState();
    _rows = widget.load(_month);
    _totals = widget.loadTotals(_month);
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
      _totals = widget.loadTotals(_month);
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
      _totals = widget.loadTotals(_month);
    });
  }

  void _step(int months) {
    setState(() {
      // The rolling lives in `dates.dart`, so this screen and the expense list
      // step months by the same arithmetic rather than by two copies of it.
      _month = addMonths(_month, months);
      _rows = widget.load(_month);
      _totals = widget.loadTotals(_month);
      _note = null;
    });
  }

  Widget _setBudgetButton() => FilledButton.tonalIcon(
        onPressed: () => _setBudget(),
        icon: const Icon(Icons.add),
        label: const Text('Set a budget'),
      );

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    // Measured, not assumed: what the table has to fit into is the width this
    // screen was handed, which is the window less whichever navigation
    // component the shell put beside it — up to 360dp of standard drawer.
    return LayoutBuilder(
      builder: (context, constraints) {
        final bool table = constraints.maxWidth >= _tableWidth;

        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 8, 16, 8),
              child: Column(
                children: [
                  Row(
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
                      // Two steppers and a labelled button come to 340dp of
                      // furniture before the month is written at all, which
                      // on a 360dp phone left "August 2026" a letter wide and
                      // spelled downwards. Narrow, the button takes the line
                      // below rather than the month's room on this one.
                      if (table) ...[
                        const SizedBox(width: 8),
                        _setBudgetButton(),
                      ],
                    ],
                  ),
                  if (!table)
                    Align(
                      alignment: Alignment.centerRight,
                      child: _setBudgetButton(),
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
                    // Nothing reserved at the end: the new-expense button does
                    // not float over this screen. Setting a budget is what
                    // this screen is for and it has its own button up in the
                    // header, so the last row can end where the list ends.
                    children: [
                      for (final row in budgeted) ...[
                        _BudgetLine(
                          row: row,
                          table: table,
                          onEdit: () => _setBudget(row),
                          onRecordSaved: () => _recordSaved(row),
                        ),
                        // Stacked rows are three lines deep and would run into
                        // each other as one paragraph. A table row is one line
                        // and needs no rule under it.
                        if (!table) const Divider(height: 1),
                      ],
                      _Totals(totals: _totals, table: table),
                      if (without > 0)
                        _Message(
                          '$without '
                          '${without == 1 ? 'category has' : 'categories have'}'
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
      },
    );
  }
}

/// One category's line: a table row where there is room, a stacked item where
/// there is not.
///
/// The stacked form is not the table with columns shaved off it. A phone is
/// 360dp wide and the table's fixed columns are 600 of them, so trimming ends
/// with the name squeezed to nothing and the figures touching — which is worse
/// than an overflow, because nothing on screen says it happened. Instead the
/// name takes a line of its own and the figures carry their own labels.
class _BudgetLine extends StatelessWidget {
  const _BudgetLine({
    required this.row,
    required this.table,
    required this.onEdit,
    required this.onRecordSaved,
  });

  final BudgetRow row;
  final bool table;
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

    final Widget name = Text(
      '${row.groupName} / ${row.categoryName}',
      style: theme.textTheme.bodyLarge,
      overflow: TextOverflow.ellipsis,
    );

    // Stacked, the group goes above the name instead of in front of it.
    // "Non-Monthly / Home Maintenance & Repairs" on a 360dp phone ellipsised
    // to "Non-Monthly / Home Ma…" — the prefix is the same on every row of a
    // group and it was eating the half of the line that says which row this
    // is. An M3 overline gives the identifying part the full width.
    final Widget stackedName = Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          row.groupName,
          style: theme.textTheme.labelSmall?.copyWith(
            color: theme.colorScheme.onSurfaceVariant,
          ),
          overflow: TextOverflow.ellipsis,
        ),
        Text(
          row.categoryName,
          style: theme.textTheme.bodyLarge,
          overflow: TextOverflow.ellipsis,
        ),
      ],
    );

    // Spent this month and spent this cycle are the same number whenever the
    // cycle is the month, which is every ordinary envelope. Only a cycle that
    // outlasts the month makes them two different facts worth two figures —
    // and only when something actually went out this month, for the same
    // reason "Over" is hidden at zero: a row of "Spent (month) $0.00" down
    // every sinking fund is a phone's width spent saying nothing happened.
    final bool cycleOutlastsMonth = row.spentCycle != null &&
        row.spentThisMonth.value != 0 &&
        row.spentThisMonth.value != row.spentCycle!.value;

    // Stacked, the labelled figures above already carry every number this
    // sentence repeats: it was printing "spent $495.37" directly under
    // "Spent (cycle) $495.37". It earns its line only where it says something
    // the figures cannot — a bill is behind by what it still owes.
    final bool showFunding =
        table || (row.groupIsObligation && row.funding != 'none');

    return InkWell(
      onTap: onEdit,
      child: Container(
        // Keyed so a test can ask where this row's parts actually are: the
        // complaint it exists to answer is figures on top of each other, and
        // nothing but their rectangles can settle that.
        key: Key('budget-row-${row.categoryId}'),
        color: background,
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (table)
              Row(
                children: [
                  Expanded(child: name),
                  _Cell(label: 'Per month', amount: row.perMonth),
                  _Cell(label: 'Funded', amount: row.funded),
                  _Cell(label: 'Spent (cycle)', amount: row.spentCycle),
                  _Cell(label: 'Left', amount: row.remaining),
                  // Positive and unsigned, and an em dash rather than $0.00:
                  // zero over budget is not a figure, it is the absence of one.
                  _Cell(
                    label: 'Over',
                    amount: over ? row.overspentBy : null,
                    colour: theme.colorScheme.error,
                  ),
                ],
              )
            else ...[
              stackedName,
              const SizedBox(height: 6),
              // Wrapped rather than columned: each figure carries its label, so
              // a second run costs a line instead of stranding numbers under a
              // heading that scrolled off. Per month and Funded are what stays
              // behind — they are the plan rather than how it is going, and
              // tapping the row opens the editor that owns them.
              Wrap(
                spacing: 20,
                runSpacing: 2,
                children: [
                  // First and heaviest. "How much is left" is the question the
                  // screen gets opened for; the rest is what the answer means.
                  _Figure(label: 'Left', amount: row.remaining, strong: true),
                  _Figure(label: 'Spent (cycle)', amount: row.spentCycle),
                  // Only where the cycle runs past the month, which is the
                  // only case where this is not the figure beside it again.
                  if (cycleOutlastsMonth)
                    _Figure(
                      label: 'Spent (month)',
                      amount: row.spentThisMonth,
                    ),
                  // Only when there is one. Over is a negative Left in
                  // positive clothes — the core defines it as that figure's
                  // absolute value — so a standing "Over —" would spend a
                  // phone's width saying there is no number to show.
                  if (over)
                    _Figure(
                      label: 'Over',
                      amount: row.overspentBy,
                      colour: theme.colorScheme.error,
                    ),
                ],
              ),
            ],
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
            if (showFunding)
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

/// A heading over a figure: one cell of the table.
class _Cell extends StatelessWidget {
  const _Cell({required this.label, required this.amount, this.colour});

  final String label;
  final Cents? amount;

  /// Applied to the figure and never to the em dash: a colour on "there is no
  /// number here" is a colour on nothing.
  final Color? colour;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return SizedBox(
      width: _columnWidth,
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
                  style: theme.textTheme.bodyMedium?.copyWith(color: colour),
                ),
        ],
      ),
    );
  }
}

/// A figure with its name beside it, for the stacked layout.
///
/// The label travels with the number because there is no column heading to
/// look up to. A bare $104.63 under a category name could be any of the five
/// things this screen knows about that category, and guessing which is exactly
/// the mistake the table's headings exist to prevent.
class _Figure extends StatelessWidget {
  const _Figure({
    required this.label,
    required this.amount,
    this.colour,
    this.strong = false,
  });

  final String label;
  final Cents? amount;
  final Color? colour;

  /// The figure the row is read for, rather than the ones giving it context.
  final bool strong;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final TextStyle? figure =
        (strong ? theme.textTheme.titleSmall : theme.textTheme.bodyMedium)
            ?.copyWith(color: colour);

    return Row(
      mainAxisSize: MainAxisSize.min,
      // Baselines rather than centres: an 11dp label centred against a 14dp
      // figure rides high and reads as a superscript on the amount.
      crossAxisAlignment: CrossAxisAlignment.baseline,
      textBaseline: TextBaseline.alphabetic,
      children: [
        Text(
          label,
          style: theme.textTheme.labelSmall?.copyWith(
            color: theme.colorScheme.onSurfaceVariant,
          ),
        ),
        const SizedBox(width: 6),
        amount == null
            ? Text('—', style: figure)
            : MoneyText(amount!, style: figure),
      ],
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


/// The table's bottom line.
///
/// Summed in the core, from the same rows the table renders. Not added up
/// here: `Cents` exists to make arithmetic a compile error, so a total on this
/// screen can never be a figure the frontend invented.
class _Totals extends StatelessWidget {
  const _Totals({required this.totals, required this.table});

  final Future<BudgetTotals> totals;
  final bool table;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return FutureBuilder<BudgetTotals>(
      future: totals,
      builder: (context, snapshot) {
        final data = snapshot.data;
        // Nothing at all while it loads, rather than a row of zeroes: a bottom
        // line reading $0.00 is a claim, and briefly making a false one is
        // worse than briefly making none.
        if (data == null) return const SizedBox.shrink();

        // Overspending in one envelope is not cancelled by another having
        // room, so this is the sum of what is actually over rather than a net
        // figure — and it is therefore *not* the mirror of Left the way a
        // single row's is. Both are shown wherever either is.
        final bool over = data.overspentBy.value != 0;

        final Widget label = Text(
          'Total of ${data.categories} '
          '${data.categories == 1 ? 'category' : 'categories'}',
          style: theme.textTheme.titleSmall,
        );

        // No column headings. The table above already carries them, and a
        // footer that repeats "Spent (cycle)" under "Spent (cycle)" reads as a
        // second table rather than the sum of the first.
        Widget figure(Cents amount, {Color? colour}) => SizedBox(
              width: _columnWidth,
              child: MoneyText(
                amount,
                textAlign: TextAlign.right,
                style: theme.textTheme.titleSmall?.copyWith(color: colour),
              ),
            );

        return Container(
          key: const Key('budget-totals'),
          color: theme.colorScheme.surfaceContainerHigh,
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
          child: table
              ? Row(
                  children: [
                    Expanded(child: label),
                    figure(data.perMonth),
                    figure(data.funded),
                    figure(data.spentCycle),
                    figure(data.remaining),
                    SizedBox(
                      width: _columnWidth,
                      child: Align(
                        alignment: Alignment.centerRight,
                        // Zero reads as an em dash for the same reason as the
                        // rows: no overspend is the absence of a number, not a
                        // number.
                        child: over
                            ? MoneyText(
                                data.overspentBy,
                                textAlign: TextAlign.right,
                                style: theme.textTheme.titleSmall
                                    ?.copyWith(color: theme.colorScheme.error),
                              )
                            : Text('—', style: theme.textTheme.titleSmall),
                      ),
                    ),
                  ],
                )
              : Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    label,
                    const SizedBox(height: 6),
                    // Labelled here, unlike the table's footer: stacked there
                    // are no column headings above to inherit, and the same
                    // three figures in the same order as the rows is what makes
                    // this read as their sum.
                    Wrap(
                      spacing: 20,
                      runSpacing: 2,
                      children: [
                        _Figure(
                          label: 'Left',
                          amount: data.remaining,
                          strong: true,
                        ),
                        _Figure(
                          label: 'Spent (cycle)',
                          amount: data.spentCycle,
                        ),
                        if (over)
                          _Figure(
                            label: 'Over',
                            amount: data.overspentBy,
                            colour: theme.colorScheme.error,
                          ),
                      ],
                    ),
                  ],
                ),
        );
      },
    );
  }
}

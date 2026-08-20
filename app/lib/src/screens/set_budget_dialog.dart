import 'package:flutter/material.dart';

import '../api.dart';
import '../dates.dart';
import '../money.dart';

typedef BudgetSetter = Future<Superseded> Function({
  required int categoryId,
  required Cents amount,
  required BudgetPeriod period,
  required String month,
  bool rollover,
});

/// Set a category's budget.
///
/// # A budget is a historical record, not a setting
///
/// Changing one does not rewrite what last year was budgeted at: the core
/// closes the line that was in force and opens a new one from this month. That
/// is why this dialog asks which month it starts from without making a fuss
/// about it, and why the result says what it did.
class SetBudgetDialog extends StatefulWidget {
  const SetBudgetDialog({
    super.key,
    required this.categories,
    required this.month,
    this.save = setBudget,
    this.initialCategoryId,
  });

  final List<CategoryDto> categories;

  /// The month the line starts from, as `YYYY-MM`.
  final String month;

  final BudgetSetter save;
  final int? initialCategoryId;

  @override
  State<SetBudgetDialog> createState() => _SetBudgetDialogState();
}

class _SetBudgetDialogState extends State<SetBudgetDialog> {
  final TextEditingController _amount = TextEditingController();
  late int? _categoryId = widget.initialCategoryId;
  BudgetPeriod _period = BudgetPeriod.monthly;
  bool _saving = false;
  String? _problem;

  @override
  void dispose() {
    _amount.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    final int? categoryId = _categoryId;
    final Cents? amount = parseCents(_amount.text);
    if (categoryId == null) {
      setState(() => _problem = 'Pick a category to budget for.');
      return;
    }
    if (amount == null) {
      // Not zero: an unparseable amount is a half-typed one, and a $0.00
      // budget is a claim that this category should have nothing.
      setState(() => _problem = 'Enter an amount, like 250.00.');
      return;
    }

    setState(() {
      _saving = true;
      _problem = null;
    });
    try {
      final result = await widget.save(
        categoryId: categoryId,
        amount: amount,
        period: _period,
        month: widget.month,
      );
      if (mounted) Navigator.of(context).pop(result);
    } catch (error) {
      if (mounted) {
        setState(() {
          _problem = '$error';
          _saving = false;
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return AlertDialog(
      title: const Text('Set a budget'),
      content: SizedBox(
        width: 420,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            DropdownButtonFormField<int>(
              initialValue: _categoryId,
              isExpanded: true,
              decoration: const InputDecoration(
                labelText: 'Category',
                border: OutlineInputBorder(),
              ),
              hint: const Text('Choose a category…'),
              items: [
                for (final category in widget.categories)
                  DropdownMenuItem<int>(
                    value: category.id,
                    child: Text('${category.groupName} / ${category.name}'),
                  ),
              ],
              onChanged: (id) => setState(() => _categoryId = id),
            ),
            const SizedBox(height: 16),
            TextField(
              controller: _amount,
              autofocus: true,
              keyboardType: const TextInputType.numberWithOptions(decimal: true),
              onSubmitted: (_) => _save(),
              decoration: const InputDecoration(
                labelText: 'Amount',
                prefixText: r'$',
                hintText: '250.00',
                border: OutlineInputBorder(),
              ),
            ),
            const SizedBox(height: 16),
            SegmentedButton<BudgetPeriod>(
              segments: [
                for (final period in BudgetPeriod.values)
                  ButtonSegment<BudgetPeriod>(
                    value: period,
                    label: Text(period.label),
                  ),
              ],
              selected: {_period},
              onSelectionChanged: (chosen) =>
                  setState(() => _period = chosen.first),
            ),
            const SizedBox(height: 8),
            Text(
              _period == BudgetPeriod.yearly
                  // The whole bill, saved for in twelve parts — which is what
                  // keeps an annual insurance premium from wrecking one month.
                  ? 'The whole year’s amount. It is saved for across twelve '
                      r'months, so a $1,200 bill is about $100 a month.'
                  : 'From ${monthLabel(widget.month)} onward. Earlier months '
                      'keep the budget they had.',
              style: theme.textTheme.bodySmall
                  ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
            ),
            if (_problem != null) ...[
              const SizedBox(height: 12),
              Text(
                _problem!,
                style: theme.textTheme.bodyMedium
                    ?.copyWith(color: theme.colorScheme.error),
              ),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: _saving ? null : () => _save(),
          child: const Text('Save budget'),
        ),
      ],
    );
  }
}

/// What to say after a budget is set.
String budgetOutcome(Superseded result, String category) {
  if (result.restated) return 'Updated the budget for $category.';
  final closed = result.closedAfter;
  return closed == null
      ? 'Budgeted $category.'
      // Naming the month the old line now ends in, because that is the part a
      // user cannot see and might not expect.
      : 'Budgeted $category. The previous budget now ends ${monthLabel(closed)}.';
}

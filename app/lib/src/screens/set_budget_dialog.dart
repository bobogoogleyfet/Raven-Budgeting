import 'package:flutter/material.dart';

import '../api.dart';
import '../dates.dart';
import '../money.dart';
import '../widgets/cents_field.dart';
import 'new_category_dialog.dart';

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
    this.canAddCategory = true,
  });

  final List<CategoryDto> categories;

  /// The month the line starts from, as `YYYY-MM`.
  final String month;

  final BudgetSetter save;
  final int? initialCategoryId;

  /// Whether the category list can be added to from here. Off in tests that
  /// are about budgets rather than about categories.
  final bool canAddCategory;

  @override
  State<SetBudgetDialog> createState() => _SetBudgetDialogState();
}

class _SetBudgetDialogState extends State<SetBudgetDialog> {
  final TextEditingController _amount = TextEditingController();
  late int? _categoryId = widget.initialCategoryId;
  late List<CategoryDto> _categories = widget.categories;
  BudgetPeriod _period = BudgetPeriod.monthly;
  bool _saving = false;
  String? _problem;

  @override
  void dispose() {
    _amount.dispose();
    super.dispose();
  }

  /// Add a category, and select it — nobody opens this to make a category and
  /// then wants to go looking for it.
  Future<void> _newCategory() async {
    final created = await showDialog<CategoryDto>(
      context: context,
      builder: (context) => const NewCategoryDialog(),
    );
    if (created == null || !mounted) return;
    setState(() {
      _categories = [..._categories, created];
      _categoryId = created.id;
      _problem = null;
    });
  }

  Future<void> _save() async {
    final int? categoryId = _categoryId;
    final Cents? amount = amountValue(_amount.text);
    if (categoryId == null) {
      setState(() => _problem = 'Pick a category to budget for.');
      return;
    }
    if (amount == null) {
      // Not zero: an unparseable amount is a half-typed one, and a $0.00
      // budget is a claim that this category should have nothing.
      setState(() => _problem = 'Enter an amount.');
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
            Row(
              children: [
                Expanded(
                  child: DropdownButtonFormField<int>(
                    initialValue: _categoryId,
                    isExpanded: true,
                    decoration: const InputDecoration(
                      labelText: 'Category',
                      border: OutlineInputBorder(),
                    ),
                    hint: const Text('Choose a category…'),
                    items: [
                      for (final category in _categories)
                        DropdownMenuItem<int>(
                          value: category.id,
                          child:
                              Text('${category.groupName} / ${category.name}'),
                        ),
                    ],
                    onChanged: (id) => setState(() => _categoryId = id),
                  ),
                ),
                if (widget.canAddCategory) ...[
                  const SizedBox(width: 8),
                  // Here rather than only in some settings screen: the moment
                  // you notice a category is missing is the moment you are
                  // trying to budget for it.
                  IconButton.filledTonal(
                    onPressed: _newCategory,
                    icon: const Icon(Icons.add),
                    tooltip: 'New category',
                  ),
                ],
              ],
            ),
            const SizedBox(height: 16),
            CentsField(
              controller: _amount,
              autofocus: true,
              label: 'Amount',
              onSubmitted: (_) => _save(),
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

import 'package:flutter/material.dart';

import '../api.dart';
import '../money.dart';
import '../widgets/cents_field.dart';
import '../widgets/money_text.dart';

/// One line of the receipt as the dialog holds it.
class ReviewLine {
  ReviewLine({
    required this.categoryId,
    required this.label,
    required this.subtotalText,
    required this.taxText,
  });

  final int? categoryId;
  final String label;
  String subtotalText;
  String taxText;

  /// Whether the tax figure is the user's rather than the app's.
  ///
  /// Only the app's are refreshed when something else changes: raising one
  /// row's amount moves the tax on every row that shares its rate, and a
  /// corrected figure must survive that.
  bool taxEdited = false;
}

/// The lines a preview of the untouched draft starts from.
List<ReviewLine> linesFrom(Preview preview) => [
      for (final row in preview.rows)
        ReviewLine(
          categoryId: row.categoryId,
          label: row.categoryName ?? '— no category —',
          subtotalText: formatCents(row.subtotal),
          taxText: formatCents(row.tax),
        ),
    ];

/// The lines as the core wants them.
List<DraftRow> editsFrom(List<ReviewLine> lines) => [
      for (final line in lines)
        DraftRow(
          categoryId: line.categoryId,
          subtotal: amountValue(line.subtotalText) ?? zero,
          // Only a figure the user actually touched is sent as an override.
          // Sending every row's tax back would make each save look like it had
          // been read off the paper, and `taxIsEstimated` would stop meaning
          // anything.
          tax: line.taxEdited ? (amountValue(line.taxText) ?? zero) : null,
        ),
    ];

/// The "is this right?" step, between pressing Save and writing anything.
///
/// The app works the tax out from a published rate, and a published rate is an
/// estimate: it is right for most receipts and wrong for the ones with a
/// bag-fee, a bottle deposit, a coupon applied after tax, or a jurisdiction the
/// quarterly file has not caught up with. The receipt in the user's hand is the
/// evidence, and this is where the two are put side by side while the paper is
/// still in front of them.
///
/// So every category's amount and tax is editable here, and each edit goes back
/// through the same preview the form uses. The subtotal, the total tax and the
/// total are *not* editable: they are sums, and a sum you can type is a sum
/// that can disagree with its parts.
///
/// What the user wants to happen once this expense is written.
///
/// Asked here rather than after saving, because this dialog is already the
/// moment they decide the receipt is finished — a second prompt afterwards
/// would interrupt the same decision twice, and a silent choice would be wrong
/// half the time. Somebody working through a pile of receipts and somebody
/// entering the one in their pocket want opposite things, and neither is rare
/// enough to be the exception.
enum SaveThen {
  /// Stay on the form, cleared and focused, for the next receipt.
  another,

  /// Close the form and go back to the overview.
  home,
}

/// An approved expense, and where the user wants to go once it is saved.
class ReviewOutcome {
  const ReviewOutcome(this.rows, this.then);

  final List<DraftRow> rows;
  final SaveThen then;
}

/// Returns the approved rows and what to do next, or null if the user said no.
Future<ReviewOutcome?> showReviewDialog({
  required BuildContext context,
  required Preview preview,
  required Future<Preview> Function(List<DraftRow> edits) recompute,
}) {
  return showDialog<ReviewOutcome>(
    context: context,
    builder: (context) => _ReviewDialog(preview: preview, recompute: recompute),
  );
}

class _ReviewDialog extends StatefulWidget {
  const _ReviewDialog({required this.preview, required this.recompute});

  final Preview preview;
  final Future<Preview> Function(List<DraftRow> edits) recompute;

  @override
  State<_ReviewDialog> createState() => _ReviewDialogState();
}

class _ReviewDialogState extends State<_ReviewDialog> {
  late Preview _preview = widget.preview;
  late final List<ReviewLine> _lines = linesFrom(widget.preview);
  late final List<TextEditingController> _tax = [
    for (final line in _lines) TextEditingController(text: line.taxText),
  ];
  late final List<TextEditingController> _subtotal = [
    for (final line in _lines) TextEditingController(text: line.subtotalText),
  ];

  /// Which recompute is the newest. A slower earlier request must not
  /// overwrite a newer answer.
  int _pending = 0;
  String? _problem;

  @override
  void dispose() {
    for (final controller in [..._tax, ..._subtotal]) {
      controller.dispose();
    }
    super.dispose();
  }

  void _refresh() {
    final int mine = ++_pending;
    widget.recompute(editsFrom(_lines)).then((fresh) {
      if (!mounted || mine != _pending) return;
      setState(() {
        _preview = fresh;
        _problem = null;
        for (int i = 0; i < _lines.length && i < fresh.rows.length; i++) {
          // A row the user has not corrected shows what the app now makes it:
          // changing one amount moves the tax on every row sharing its rate.
          if (_lines[i].taxEdited) continue;
          _lines[i].taxText = formatCents(fresh.rows[i].tax);
          // The text is replaced, not the field: rebuilding the widget the
          // user is typing in takes the caret with it.
          _tax[i].text = _lines[i].taxText;
        }
      });
    }).catchError((Object error) {
      if (!mounted) return;
      setState(() => _problem = '$error');
    });
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final bool blocked = _preview.blockers.isNotEmpty || _problem != null;
    final String? problem =
        _problem ?? (_preview.blockers.isEmpty ? null : _preview.blockers.join('; '));

    return AlertDialog(
      title: const Text('Does this match your receipt?'),
      content: SizedBox(
        width: 520,
        child: SingleChildScrollView(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                'Correct any amount or tax that differs from the paper — '
                'the receipt wins.',
                style: theme.textTheme.bodyMedium
                    ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
              ),
              const SizedBox(height: 16),
              Row(
                children: [
                  const Expanded(child: SizedBox()),
                  SizedBox(
                    width: 110,
                    child: Text('Amount',
                        textAlign: TextAlign.right,
                        style: theme.textTheme.labelMedium),
                  ),
                  const SizedBox(width: 8),
                  SizedBox(
                    width: 110,
                    child: Text('Tax',
                        textAlign: TextAlign.right,
                        style: theme.textTheme.labelMedium),
                  ),
                ],
              ),
              const Divider(),
              for (int i = 0; i < _lines.length; i++)
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: 4),
                  child: Row(
                    children: [
                      Expanded(child: Text(_lines[i].label)),
                      SizedBox(
                        width: 110,
                        child: _AmountField(
                          controller: _subtotal[i],
                          onChanged: (value) {
                            _lines[i].subtotalText = value;
                            _refresh();
                          },
                        ),
                      ),
                      const SizedBox(width: 8),
                      SizedBox(
                        width: 110,
                        child: _AmountField(
                          controller: _tax[i],
                          onChanged: (value) {
                            _lines[i].taxText = value;
                            // From here this row's figure is the user's, and
                            // stays theirs.
                            _lines[i].taxEdited = true;
                            _refresh();
                          },
                        ),
                      ),
                    ],
                  ),
                ),
              const SizedBox(height: 8),
              // Sums, and therefore not editable. The rows are what the receipt
              // says; these are what the rows come to.
              const Divider(),
              _SumRow(label: 'Subtotal', amount: _preview.subtotal),
              _SumRow(label: 'Total tax', amount: _preview.tax),
              _SumRow(label: 'Total', amount: _preview.total, grand: true),
              if (problem != null) ...[
                const SizedBox(height: 12),
                Text(
                  problem,
                  style: theme.textTheme.bodyMedium
                      ?.copyWith(color: theme.colorScheme.error),
                ),
              ],
            ],
          ),
        ),
      ),
      // Three actions, which M3 allows and this dialog earns: one of them
      // cancels and the other two are the same save with different
      // destinations.
      //
      // Kept to two words each because they are read at a glance, not because
      // it keeps them on one row — it does not. Below roughly 400px the
      // OverflowBar stacks all three vertically however short they are, which
      // is M3's own fallback and reads fine. Measured at 320 and 360px: no
      // clipping, no overflow. The length is for legibility; the stacking
      // takes care of itself.
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Back'),
        ),
        TextButton(
          onPressed: blocked
              ? null
              : () => Navigator.of(context)
                  .pop(ReviewOutcome(editsFrom(_lines), SaveThen.home)),
          child: const Text('Save & Home'),
        ),
        FilledButton(
          // Named for the action, never "OK". This one is the default because
          // the form is already open and cleared behind the dialog: carrying
          // on is the cheaper of the two, and leaving is one tap either way.
          onPressed: blocked
              ? null
              : () => Navigator.of(context)
                  .pop(ReviewOutcome(editsFrom(_lines), SaveThen.another)),
          child: const Text('Save & New'),
        ),
      ],
    );
  }
}

class _AmountField extends StatelessWidget {
  const _AmountField({required this.controller, required this.onChanged});

  final TextEditingController controller;
  final ValueChanged<String> onChanged;

  @override
  Widget build(BuildContext context) {
    return CentsField(controller: controller, onChanged: onChanged);
  }
}

class _SumRow extends StatelessWidget {
  const _SumRow({required this.label, required this.amount, this.grand = false});

  final String label;
  final Cents amount;
  final bool grand;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final style = grand
        ? theme.textTheme.titleMedium?.copyWith(fontWeight: FontWeight.w700)
        : theme.textTheme.bodyMedium;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        children: [
          Expanded(child: Text(label, style: style)),
          SizedBox(
            width: 110,
            child: MoneyText(amount, textAlign: TextAlign.right, style: style),
          ),
          const SizedBox(width: 8),
          const SizedBox(width: 110),
        ],
      ),
    );
  }
}

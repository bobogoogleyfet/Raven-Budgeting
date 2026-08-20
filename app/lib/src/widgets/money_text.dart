
import 'package:flutter/material.dart';

import '../money.dart';

/// An amount, formatted by [formatMoney] and set in tabular figures.
///
/// Tabular because a column of amounts that do not line up is a column you
/// have to read twice. This widget exists so that no screen is tempted to
/// format money its own way.
class MoneyText extends StatelessWidget {
  const MoneyText(this.amount, {super.key, this.style, this.textAlign});

  final Cents amount;
  final TextStyle? style;
  final TextAlign? textAlign;

  @override
  Widget build(BuildContext context) {
    return Text(
      formatMoney(amount),
      style: (style ?? DefaultTextStyle.of(context).style).copyWith(
        fontFeatures: const [FontFeature.tabularFigures()],
      ),
      textAlign: textAlign,
      maxLines: 1,
    );
  }
}

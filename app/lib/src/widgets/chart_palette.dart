import 'package:flutter/material.dart';

/// The colours the budget ring draws its segments with.
///
/// # Why these are not Material 3 roles
///
/// M3 names roles for an interface — primary, secondary, tertiary — and a
/// chart needs something it does not supply: a set of hues that stay told
/// apart from each other, including by a reader with colour-vision deficiency.
/// Three tertiaries do not make a categorical scale. So the interface stays M3
/// and the data marks use this, which is a separate, validated instance.
///
/// # These were validated, not chosen
///
/// Seven slots in a fixed order, checked with the palette validator against
/// this app's own surfaces (`#f7f9ff` light, `#101418` dark):
///
/// * worst adjacent CVD separation ΔE 9.1 light / 8.4 dark, against a target
///   of 8;
/// * worst adjacent normal-vision ΔE 19.6 light / 19.3 dark, against a floor
///   of 15;
/// * three light slots fall below 3:1 against the surface, which the validator
///   flags as needing relief — the legend beside the ring is that relief, and
///   is why identity never rests on colour alone here.
///
/// # Seven, not eight
///
/// The reference palette's eighth slot is a red, and red is reserved: it means
/// over budget. A status colour that also meant "category eight" would make
/// the one thing the ring must not get wrong ambiguous. Categories past the
/// seventh fold into one grey arc rather than taking a generated hue — a
/// generated eighth is indistinguishable from an existing one under CVD.
class ChartPalette {
  const ChartPalette._();

  static const List<Color> _light = [
    Color(0xFF2A78D6), // blue
    Color(0xFFEB6834), // orange
    Color(0xFF1BAF7A), // aqua
    Color(0xFFEDA100), // yellow
    Color(0xFFE87BA4), // magenta
    Color(0xFF008300), // green
    Color(0xFF4A3AA7), // violet
  ];

  static const List<Color> _dark = [
    Color(0xFF3987E5),
    Color(0xFFD95926),
    Color(0xFF199E70),
    Color(0xFFC98500),
    Color(0xFFD55181),
    Color(0xFF008300),
    Color(0xFF9085E9),
  ];

  /// How many categories get a hue of their own before the tail folds.
  static const int slots = 7;

  static List<Color> of(BuildContext context) =>
      Theme.of(context).brightness == Brightness.dark ? _dark : _light;

  /// The colour for the folded tail, and for a segment with no hue left.
  static Color other(BuildContext context) =>
      Theme.of(context).colorScheme.onSurfaceVariant;
}

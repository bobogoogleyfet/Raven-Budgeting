import 'package:flutter/material.dart';

/// The Material 3 FAB sizes, and the one Flutter does not ship.
///
/// M3 sizes the button by breakpoint rather than using one button everywhere:
/// a medium FAB for mobile layouts, a large one for tablets and larger. The
/// medium size arrived with the expressive update and Flutter has no
/// constructor for it — `FloatingActionButton` offers small, regular and
/// large — so it is built here by overriding the three tokens that differ.
///
/// The numbers are the spec's, read from the token table on
/// https://m3.material.io/components/floating-action-button/specs :
///
///   FAB           56dp container, 24dp icon, large rounding            (16dp)
///   Medium FAB    80dp container, 28dp icon, large increased rounding  (20dp)
///   Large FAB     96dp container, 36dp icon, extra large rounding      (28dp)
///
/// Only the medium size is spelled out below, because it is the only one
/// Flutter cannot make on its own.
const double kMediumFabSize = 80;
const double kMediumFabIcon = 28;
const double kMediumFabCorner = 20;

/// What a floating button costs the bottom of a scrolling list.
///
/// The button sits [kFabMargin] from the bottom edge, so the last row has to
/// travel its whole height plus that margin to get out from under it, and
/// another margin to not be touching it. Derived rather than written down as a
/// number: the three screens that reserve this space had drifted to two
/// different constants, neither of which would have survived the button
/// changing size.
const double kFabMargin = 16;
const double kFabReserve = kMediumFabSize + kFabMargin * 2;

/// A Material 3 medium FAB.
///
/// The size, icon and corner come through a [Theme] override rather than
/// through arguments because that is where `FloatingActionButton` reads them
/// from; everything not named here — the color roles, the elevation, the state
/// layers — falls through to Flutter's M3 defaults, which is the point of
/// overriding three tokens instead of building a button.
class MediumFab extends StatelessWidget {
  const MediumFab({
    super.key,
    required this.onPressed,
    required this.tooltip,
    required this.icon,
    this.elevation,
  });

  final VoidCallback onPressed;
  final String tooltip;
  final IconData icon;

  /// Left to the M3 default unless a caller has reason to flatten it, which
  /// the one inside the navigation rail does.
  final double? elevation;

  @override
  Widget build(BuildContext context) {
    final ThemeData theme = Theme.of(context);
    return Theme(
      data: theme.copyWith(
        floatingActionButtonTheme: theme.floatingActionButtonTheme.copyWith(
          sizeConstraints: const BoxConstraints.tightFor(
            width: kMediumFabSize,
            height: kMediumFabSize,
          ),
          iconSize: kMediumFabIcon,
          shape: const RoundedRectangleBorder(
            borderRadius: BorderRadius.all(Radius.circular(kMediumFabCorner)),
          ),
        ),
      ),
      child: FloatingActionButton(
        onPressed: onPressed,
        tooltip: tooltip,
        elevation: elevation,
        child: Icon(icon),
      ),
    );
  }
}

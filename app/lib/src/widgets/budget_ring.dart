import 'dart:math' as math;

import 'package:flutter/material.dart';

/// One category's arc.
///
/// The arc's *length* is that category's share of the month's budget; the
/// filled part of it is how much of that category has been spent.
class RingSegment {
  const RingSegment({
    required this.label,
    required this.budgeted,
    required this.spent,
    required this.isOver,
    required this.colour,
  });

  final String label;

  /// This category's hue, or the folded-tail grey. Over-budget overrides it
  /// with the error colour when the segment is painted: red means one thing
  /// here, and it is not "category eight".
  final Color colour;

  /// Cents. Used for geometry only — the arc's share of the circle and how far
  /// along it the fill runs. No monetary figure is derived from it.
  final int budgeted;
  final int spent;

  /// The core's own over-budget signal, not a comparison made here. Behind on
  /// funding is a different question and must not turn anything red.
  final bool isOver;
}

/// A ring of per-category meters.
///
/// Not a pie: a pie encodes one value per slice as an angle, and this encodes
/// two — the budget as the arc's length, the spending as how much of that arc
/// is filled. Each segment is a meter, and the ring is what lets several be
/// read at a glance without several separate bars.
///
/// Colour does one job here: state. A segment is the accent hue until the core
/// says that category is over budget, and then it is filled and red. Identity
/// comes from the legend beside it, never from hue — which is why the segments
/// do not each get a colour of their own.
class BudgetRing extends StatelessWidget {
  const BudgetRing({
    super.key,
    required this.segments,
    required this.centre,
    this.selected,
    this.onSelect,
    this.thickness = 20,
  });

  final List<RingSegment> segments;

  /// What sits in the hole. The ring is geometry; the figure is the point.
  final Widget centre;

  final int? selected;
  final void Function(int? index)? onSelect;
  final double thickness;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return LayoutBuilder(
      builder: (context, constraints) {
        final double size =
            math.min(constraints.maxWidth, constraints.maxHeight);
        final painter = _RingPainter(
          segments: segments,
          selected: selected,
          thickness: thickness,
          track: theme.colorScheme.outlineVariant,
          over: theme.colorScheme.error,
        );

        return GestureDetector(
          onTapUp: onSelect == null
              ? null
              : (details) => onSelect!(
                    painter.segmentAt(details.localPosition, Size(size, size)),
                  ),
          child: SizedBox(
            width: size,
            height: size,
            child: CustomPaint(
              painter: painter,
              child: Center(
                child: Padding(
                  // Inside the hole, with room to breathe: a figure that
                  // touches the ring reads as part of it.
                  padding: EdgeInsets.all(thickness + 12),
                  child: centre,
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}

class _RingPainter extends CustomPainter {
  _RingPainter({
    required this.segments,
    required this.selected,
    required this.thickness,
    required this.track,
    required this.over,
  });

  final List<RingSegment> segments;
  final int? selected;
  final double thickness;
  final Color track;
  final Color over;

  /// Twelve o'clock. A ring that starts at three reads as having been rotated.
  static const double _start = -math.pi / 2;

  int get _total => segments.fold<int>(
        0,
        (sum, segment) => sum + math.max(segment.budgeted, 0),
      );

  /// The angular width of the gap between segments: three pixels of arc.
  ///
  /// Enough to read as a separation and small enough that eight of them do not
  /// eat the circle. The caps are square, so the gap is exactly what it looks
  /// like — a round cap would overhang its own arc by half the stroke at each
  /// end, which is what turns modest gaps into touching blobs.
  double _gapFor(double radius) => radius <= 0 ? 0 : 3 / radius;

  List<({double from, double to})> _arcs(double radius) {
    final total = _total;
    if (total <= 0) return const [];

    final double gapAngle = _gapFor(radius);
    // Every segment gives up the same slice to its own gap, so the arcs stay
    // proportional to the budgets rather than to the budgets minus a constant.
    final double available = 2 * math.pi - gapAngle * segments.length;
    final arcs = <({double from, double to})>[];
    double cursor = _start + gapAngle / 2;

    for (final segment in segments) {
      final double sweep = available * (math.max(segment.budgeted, 0) / total);
      arcs.add((from: cursor, to: cursor + sweep));
      cursor += sweep + gapAngle;
    }
    return arcs;
  }

  /// Which segment a tap landed on, or null for the hole and the outside.
  int? segmentAt(Offset position, Size size) {
    final Offset centre = Offset(size.width / 2, size.height / 2);
    final double radius =
        math.min(size.width, size.height) / 2 - thickness / 2;
    if (radius <= 0) return null;

    final Offset local = position - centre;
    final double distance = local.distance;
    // A generous band: a 20px ring is a small target on a phone, where
    // Material asks for 48. Anything nearer the middle belongs to the figure.
    if (distance < radius - thickness * 1.5 || distance > radius + thickness) {
      return null;
    }

    double angle = math.atan2(local.dy, local.dx);
    while (angle < _start) {
      angle += 2 * math.pi;
    }

    final arcs = _arcs(radius);
    for (int i = 0; i < arcs.length; i++) {
      if (angle >= arcs[i].from && angle <= arcs[i].to) return i;
    }
    return null;
  }

  @override
  void paint(Canvas canvas, Size size) {
    final double radius =
        math.min(size.width, size.height) / 2 - thickness / 2;
    if (radius <= 0) return;

    final Rect box = Rect.fromCircle(
      center: Offset(size.width / 2, size.height / 2),
      radius: radius,
    );
    final arcs = _arcs(radius);

    for (int i = 0; i < arcs.length; i++) {
      final segment = segments[i];
      final arc = arcs[i];
      final double sweep = arc.to - arc.from;
      final Color colour = segment.isOver ? over : segment.colour;

      // Each segment's own colour, faint: the arc's *length* is the budget, so
      // it has to be visible before a penny is spent. A single grey track
      // would hide where one category ends and the next begins, which is the
      // whole of what the ring is for.
      canvas.drawArc(
        box,
        arc.from,
        sweep,
        false,
        Paint()
          ..color = colour.withValues(alpha: 0.22)
          ..strokeWidth = thickness
          ..style = PaintingStyle.stroke,
      );

      if (selected == i) {
        canvas.drawArc(
          box,
          arc.from,
          sweep,
          false,
          Paint()
            ..color = track
            ..strokeWidth = thickness + 5
            ..style = PaintingStyle.stroke
            ..blendMode = BlendMode.dstOver,
        );
      }

      final int budgeted = math.max(segment.budgeted, 0);
      // A ratio for geometry, clamped: it becomes an arc length, and is never
      // shown as a number or sent anywhere.
      final double progress = budgeted == 0
          ? (segment.spent > 0 ? 1 : 0)
          : (segment.spent / budgeted).clamp(0.0, 1.0);
      // Over budget fills the segment: there is no "110% of an arc", and a
      // fill that stopped short would understate the news.
      final double filled = segment.isOver ? 1 : progress;
      if (filled <= 0) continue;

      canvas.drawArc(
        box,
        arc.from,
        sweep * filled,
        false,
        Paint()
          ..color = colour
          ..strokeWidth = thickness
          // Square ends, so a part-spent segment ends on a clean radial edge
          // in line with its neighbours' edges.
          ..strokeCap = StrokeCap.butt
          ..style = PaintingStyle.stroke,
      );
    }
  }

  @override
  bool shouldRepaint(_RingPainter old) =>
      old.segments != segments ||
      old.selected != selected ||
      old.thickness != thickness ||
      old.track != track ||
      old.over != over;
}

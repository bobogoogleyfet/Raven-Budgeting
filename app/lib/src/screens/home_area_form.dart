import 'package:flutter/material.dart';

import '../api.dart';
import '../money.dart';
import '../rates.dart';
import '../widgets/combobox.dart';

typedef AreaLoader = Future<List<AreaDto>> Function();
typedef ReleaseLoader = Future<RateReleaseDto?> Function();
typedef HomeSetter = Future<void> Function({
  required String name,
  required String? areaCode,
  required int taxRatePpm,
  required int? foodRatePpm,
});

/// Asking where the user shops.
///
/// # The rate comes from the state, not from the user
///
/// Picking a place fills in its sales-tax rate. Nobody should have to look up
/// a combined state-plus-county-plus-city rate before entering a first
/// receipt. The list is every taxing jurisdiction Utah publishes, refreshed
/// from the Tax Commission's own quarterly file.
///
/// The rate is still shown rather than hidden, because it is an **estimate**:
/// rates change on the first day of every quarter, and showing one is what
/// lets a user notice it is wrong. So is the quarter it came from — a table
/// that quietly stopped updating looks exactly like a current one otherwise.
///
/// An area the list does not know asks for the rate directly, since there is
/// nothing to derive it from. Outside Utah that is every area, which is the
/// honest form of "not supported yet": the app still works, it just cannot
/// fill the number in.
///
/// The same form serves first run and settings, deliberately. It is the same
/// question, and a second arrangement of it would be a second thing to keep
/// right.
class HomeAreaForm extends StatefulWidget {
  const HomeAreaForm({
    super.key,
    required this.submitLabel,
    required this.onSaved,
    this.firstRun = false,
    this.loadAreas = listAreas,
    this.loadRelease = rateRelease,
    this.save = setHomeLocation,
  });

  final String submitLabel;
  final VoidCallback onSaved;

  /// First run says a little more, because there is nothing else on screen to
  /// explain what the question is for.
  final bool firstRun;

  final AreaLoader loadAreas;
  final ReleaseLoader loadRelease;
  final HomeSetter save;

  @override
  State<HomeAreaForm> createState() => _HomeAreaFormState();
}

class _HomeAreaFormState extends State<HomeAreaForm> {
  final TextEditingController _area = TextEditingController();
  final TextEditingController _rate = TextEditingController();

  List<AreaDto> _areas = const [];
  String _provenance = '';
  String? _feedback;
  bool _saving = false;

  /// What the picker settled on, so the rate box appears only once the name is
  /// committed — not on every keystroke, which would make the prompt flicker.
  String _committed = '';

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    _area.dispose();
    _rate.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    try {
      final areas = await widget.loadAreas();
      if (mounted) setState(() => _areas = areas);
    } catch (error) {
      if (mounted) setState(() => _feedback = 'Could not load the area list: $error');
    }
    try {
      // Not awaited with the list: a missing provenance line is cosmetic, and
      // the picker must not wait on it to become usable.
      final release = await widget.loadRelease();
      if (!mounted) return;
      setState(() => _provenance = releaseLine(
            quarter: release?.quarter,
            current: release?.current ?? false,
          ));
    } catch (_) {
      if (mounted) setState(() => _provenance = '');
    }
  }

  /// The published area with exactly this name, if there is one.
  AreaDto? get _match {
    final String needle = _committed.trim().toLowerCase();
    if (needle.isEmpty) return null;
    for (final area in _areas) {
      if (area.name.toLowerCase() == needle) return area;
    }
    return null;
  }

  Future<void> _submit() async {
    final String where = _area.text.trim();
    setState(() => _feedback = null);
    if (where.isEmpty) {
      setState(() => _feedback = 'Pick the area you shop in most, or type it in.');
      return;
    }

    final AreaDto? match = _match;
    final int? ppm = match?.taxRatePpm ?? parseRatePpm(_rate.text);
    if (ppm == null) {
      setState(() => _feedback =
          'Enter the sales-tax rate for that area, like ${widget.firstRun ? '7.25' : '7.45'}.');
      return;
    }

    setState(() => _saving = true);
    try {
      // A hand-typed area gets null for both: the user moved the choice off
      // the list, so there is no jurisdiction to follow, and carrying Utah's
      // 3% over to Boise would undertax every grocery receipt entered there.
      await widget.save(
        name: where,
        areaCode: match?.code,
        taxRatePpm: ppm,
        foodRatePpm: match?.foodRatePpm,
      );
      if (!mounted) return;
      setState(() => _feedback = 'Home is now $where.');
      widget.onSaved();
    } catch (error) {
      if (mounted) setState(() => _feedback = '$error');
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final AreaDto? match = _match;
    final bool unknown = _committed.trim().isNotEmpty && match == null;

    return ListView(
      padding: const EdgeInsets.all(16),
      children: [
        if (widget.firstRun) ...[
          Text('Where do you shop?', style: theme.textTheme.headlineSmall),
          const SizedBox(height: 8),
          Text(
            'Sales tax is worked out from where a store is, so expenses only '
            'need their subtotals. This is the default for anywhere you have '
            'not named specifically.',
            style: theme.textTheme.bodyMedium
                ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
          ),
        ] else ...[
          Text('Home area', style: theme.textTheme.titleMedium),
          const SizedBox(height: 8),
          Text(
            "Every expense is taxed at this area's rate unless it names another "
            'location. Changing it affects what the next expense estimates, '
            'never what is already filed.',
            style: theme.textTheme.bodyMedium
                ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
          ),
        ],
        const SizedBox(height: 16),
        Combobox(
          controller: _area,
          options: [for (final area in _areas) area.name],
          label: 'Home area',
          hint: widget.firstRun ? 'Start typing your city…' : 'Select a location',
          emptyLabel: 'No areas loaded.',
          onCommit: (name) => setState(() => _committed = name),
        ),
        const SizedBox(height: 12),
        if (match != null)
          Text(
            'Sales tax ${rateSummary(match.taxRatePpm, match.foodRatePpm)}'
            '${widget.firstRun ? " — an estimate; a receipt's own tax always wins." : ''}',
            style: theme.textTheme.bodyMedium,
          ),
        if (unknown) ...[
          // Nothing to derive from, so ask. Outside Utah this is every area,
          // and it is the escape hatch that keeps "not supported yet" from
          // meaning "unusable".
          Text(
            "Not in Utah's list — what is the sales tax there?",
            style: theme.textTheme.bodyMedium
                ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              SizedBox(
                width: 120,
                child: TextField(
                  controller: _rate,
                  keyboardType:
                      const TextInputType.numberWithOptions(decimal: true),
                  onSubmitted: (_) => _submit(),
                  decoration: InputDecoration(
                    hintText: widget.firstRun ? '7.25' : '7.45',
                    isDense: true,
                    border: const OutlineInputBorder(),
                    suffixText: '%',
                  ),
                ),
              ),
            ],
          ),
        ],
        const SizedBox(height: 20),
        Row(
          children: [
            FilledButton(
              onPressed: _saving ? null : () => _submit(),
              child: Text(widget.submitLabel),
            ),
            const SizedBox(width: 16),
            Expanded(
              child: Semantics(
                liveRegion: true,
                child: Text(
                  _feedback ?? '',
                  style: theme.textTheme.bodyMedium
                      ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
                ),
              ),
            ),
          ],
        ),
        if (_provenance.isNotEmpty) ...[
          const SizedBox(height: 24),
          Text(
            _provenance,
            style: theme.textTheme.bodySmall
                ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
          ),
        ],
        if (widget.firstRun) ...[
          const SizedBox(height: 12),
          Text(
            'You can change this any time. Rates here are starting points, not '
            'gospel — they change. Groceries are taxed at the lower rate where '
            'the state has one; a restaurant meal is not, because prepared food '
            'is taxed in full.',
            style: theme.textTheme.bodySmall
                ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
          ),
        ],
      ],
    );
  }
}

import 'dart:typed_data' show Uint8List;

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';

import '../api.dart';
import '../browser/browser.dart';
import '../money.dart';
import '../rates.dart';
import '../widgets/combobox.dart';

typedef AreaLoader = Future<List<AreaDto>> Function();
typedef ReleaseLoader = Future<RateReleaseDto?> Function();
typedef RateRefresher = Future<RefreshDto> Function();
typedef WorkbookInstaller = Future<RefreshDto> Function(Uint8List bytes);
typedef HomeSetter = Future<void> Function({
  required String name,
  required String? areaCode,
  required int taxRatePpm,
  required int? foodRatePpm,
  required int? restaurantRatePpm,
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
    this.refresh = refreshRates,
    this.save = setHomeLocation,
    this.embedded = false,
    this.canRefresh = !kIsWeb,
    this.install,
    this.pick = pickFile,
  });

  final String submitLabel;
  final VoidCallback onSaved;

  /// Whether this form is one section of a longer page rather than the whole
  /// of one. Embedded, it stops scrolling on its own and lets the page it sits
  /// in do it — two scrolling regions on one settings page is how a user ends
  /// up unable to reach the section underneath.
  final bool embedded;

  /// First run says a little more, because there is nothing else on screen to
  /// explain what the question is for.
  final bool firstRun;

  final AreaLoader loadAreas;
  final ReleaseLoader loadRelease;
  final RateRefresher refresh;

  /// Whether this build can fetch a rate table at all. A browser cannot: the
  /// Tax Commission's server sends no CORS headers, and the page's
  /// Content-Security-Policy forbids talking to anything but its own origin.
  /// A button that could only ever fail is not offered. Instead the user
  /// imports the workbook themselves, through [install] and [pick].
  final bool canRefresh;

  /// Installs a rate workbook the user downloaded, as this quarter's table.
  /// Offered only where [canRefresh] is false, and only when there is one.
  final WorkbookInstaller? install;
  final FilePicker pick;
  final HomeSetter save;

  @override
  State<HomeAreaForm> createState() => _HomeAreaFormState();
}

class _HomeAreaFormState extends State<HomeAreaForm> {
  final TextEditingController _area = TextEditingController();
  final TextEditingController _rate = TextEditingController();

  List<AreaDto> _areas = const [];
  String _provenance = '';
  String? _refreshNote;
  bool _refreshing = false;
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

  /// Ask for a newer rate table now, rather than waiting for the next launch.
  ///
  /// The app already tries once a day in the background. This is for the case
  /// the provenance line makes visible: the quarter has turned, this machine
  /// has been offline, and the user is looking straight at the stale figure.
  Future<void> _checkNow() async {
    setState(() {
      _refreshing = true;
      _refreshNote = null;
    });
    try {
      final result = await widget.refresh();
      final release = await widget.loadRelease();
      if (!mounted) return;
      setState(() {
        _provenance = releaseLine(
          quarter: release?.quarter,
          current: release?.current ?? false,
        );
        _refreshNote = switch (result) {
          RefreshDto(installed: true, :final areas) =>
            'Installed ${release?.quarter ?? 'the new table'} — $areas jurisdictions.',
          RefreshDto(attempted: false) => 'Already up to date.',
          RefreshDto(problem: final String problem) => 'Could not fetch it: $problem',
          _ => 'Could not fetch it.',
        };
      });
    } catch (error) {
      if (mounted) setState(() => _refreshNote = 'Could not fetch it: $error');
    } finally {
      if (mounted) setState(() => _refreshing = false);
    }
  }

  /// Install a workbook the user fetched themselves — the browser's only way
  /// to a newer table.
  Future<void> _importWorkbook(WorkbookInstaller install) async {
    // Before anything is awaited: a browser opens its picker only from a
    // gesture it can still see.
    final Future<PickedFile?> picking = widget.pick();
    final PickedFile? file;
    try {
      file = await picking;
    } catch (error) {
      if (mounted) setState(() => _refreshNote = 'Could not read the file: $error');
      return;
    }
    // Closing the picker is a change of mind, not a failure.
    if (file == null || !mounted) return;

    setState(() {
      _refreshing = true;
      _refreshNote = null;
    });
    try {
      final result = await install(file.bytes);
      final release = await widget.loadRelease();
      if (!mounted) return;
      setState(() {
        _provenance = releaseLine(
          quarter: release?.quarter,
          current: release?.current ?? false,
        );
        _refreshNote = 'Installed ${release?.quarter ?? 'the new table'} '
            'from ${file!.name} — ${result.areas} jurisdictions.';
      });
    } catch (error) {
      // The core refuses a workbook that is not this quarter's, and says so;
      // that sentence is the whole of what the user needs.
      if (mounted) setState(() => _refreshNote = 'Could not install it: $error');
    } finally {
      if (mounted) setState(() => _refreshing = false);
    }
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
      // A hand-typed area gets null for all of them: the user moved the
      // choice off the list, so there is no jurisdiction to follow. Carrying
      // Utah's 3% over to Boise would undertax every grocery receipt entered
      // there, and carrying Utah County's restaurant tax would overtax every
      // meal.
      await widget.save(
        name: where,
        areaCode: match?.code,
        taxRatePpm: ppm,
        foodRatePpm: match?.foodRatePpm,
        restaurantRatePpm: match?.restaurantRatePpm,
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
      shrinkWrap: widget.embedded,
      physics:
          widget.embedded ? const NeverScrollableScrollPhysics() : null,
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
          hint: 'Start typing your city…',
          emptyLabel: 'Start typing to find your city.',
          // The same search the entry form uses. Not strict, though: this one
          // has the hand-typed rate underneath it, which is the honest form of
          // "outside Utah is not supported yet" — the app still works, it just
          // cannot fill the number in.
          openOnFocus: false,
          onCommit: (name) => setState(() => _committed = name),
        ),
        const SizedBox(height: 12),
        if (match != null)
          Text(
            'Sales tax ${rateSummary(match.taxRatePpm, match.foodRatePpm, match.restaurantRatePpm)}'
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
        const SizedBox(height: 24),
        Row(
          children: [
            if (_provenance.isNotEmpty)
              Flexible(
                child: Text(
                  _provenance,
                  style: theme.textTheme.bodySmall
                      ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
                ),
              ),
            if (widget.canRefresh) ...[
              const SizedBox(width: 12),
              TextButton(
                onPressed: _refreshing ? null : () => _checkNow(),
                child: Text(_refreshing ? 'Checking…' : 'Check for new rates'),
              ),
            ] else if (widget.install case final install?) ...[
              const SizedBox(width: 12),
              TextButton(
                onPressed: _refreshing ? null : () => _importWorkbook(install),
                child: Text(_refreshing ? 'Installing…' : 'Import rate workbook…'),
              ),
            ],
          ],
        ),
        if (!widget.canRefresh && widget.install != null)
          Text(
            'Raven cannot download the rate table from here. Each quarter the '
            'Utah State Tax Commission publishes it as a spreadsheet on '
            'tax.utah.gov; download it and import it with the button above.',
            style: theme.textTheme.bodySmall
                ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
          ),
        if (_refreshNote != null)
          Semantics(
            liveRegion: true,
            child: Text(
              _refreshNote!,
              style: theme.textTheme.bodySmall
                  ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
            ),
          ),
        if (widget.firstRun) ...[
          const SizedBox(height: 12),
          Text(
            'You can change this any time. Rates here are starting points, not '
            'gospel — they change. Groceries are taxed at the lower rate where '
            'the state has one; a restaurant meal is taxed higher, because most '
            'Utah counties add a restaurant tax on prepared food.',
            style: theme.textTheme.bodySmall
                ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
          ),
        ],
      ],
    );
  }
}

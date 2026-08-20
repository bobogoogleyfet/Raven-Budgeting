import 'dart:async';

import 'package:flutter/material.dart';

import '../api.dart' as api;
import '../api.dart'
    show
        CategoryDto,
        ContextDto,
        DraftRow,
        LocationDto,
        Preview,
        SubtotalDraft;
import '../dates.dart';
import '../money.dart';
import '../widgets/cents_field.dart';
import '../widgets/combobox.dart';
import '../widgets/date_field.dart';
import 'review_dialog.dart';

/// How long to wait after a keystroke before asking the core.
///
/// The preview runs on every change, and each one is a round trip plus a
/// SQLite read. Without this, typing "154.32" fires six of them and the
/// answers can land out of order, so the panel briefly shows the numbers for
/// "154.3" after the ones for "154.32".
const Duration _settle = Duration(milliseconds: 150);

/// The core calls this screen makes, in one place so a test can stand in for
/// them without a database.
class EntryApi {
  const EntryApi({
    this.listCategories = api.listCategories,
    this.listMerchants = api.listMerchants,
    this.listLocations = api.listLocations,
    this.listAreas = api.listAreas,
    this.entryContext = api.entryContext,
    this.ensureLocation = api.ensureLocation,
    this.previewSubtotals = api.previewSubtotals,
    this.saveSubtotals = api.saveSubtotals,
  });

  final Future<List<CategoryDto>> Function() listCategories;
  final Future<List<api.MerchantDto>> Function() listMerchants;
  final Future<List<LocationDto>> Function() listLocations;
  final Future<List<api.AreaDto>> Function() listAreas;
  final Future<ContextDto> Function(String? merchant) entryContext;
  final Future<LocationDto> Function(String name) ensureLocation;
  final Future<Preview> Function(SubtotalDraft draft) previewSubtotals;
  final Future<int> Function(SubtotalDraft draft) saveSubtotals;
}

/// One entered line: a category and what it came to before tax.
class _RowState {
  _RowState();

  int? categoryId;
  final TextEditingController category = TextEditingController();
  final TextEditingController amount = TextEditingController();
  final FocusNode categoryFocus = FocusNode();
  final FocusNode amountFocus = FocusNode();

  void dispose() {
    category.dispose();
    amount.dispose();
    categoryFocus.dispose();
    amountFocus.dispose();
  }
}

/// The receipt entry form.
///
/// # You type what the receipt shows, and nothing else
///
/// A receipt gives you a subtotal per category. It also gives you a tax line
/// and a total, but both of those are *derivable* — and asking for a number the
/// app can work out means the two can disagree, which is then the user's
/// problem to adjudicate. So this form asks for the parts only: tax comes from
/// the location's sales-tax rate, and the total is the sum plus that tax.
///
/// # The numbers are never this file's opinion
///
/// Every change sends the rows to the core, which reconciles the amounts and
/// apportions the tax and returns exactly what a save would write. The
/// confirmation dialog renders that answer verbatim, so the figures on screen
/// are not the app's reading of the receipt — they *are* what will be stored.
///
/// That is also why this file contains no arithmetic. It never adds two
/// subtotals and never applies a rate; it asks, and it renders.
class EntryScreen extends StatefulWidget {
  const EntryScreen({super.key, this.onSaved, this.core = const EntryApi()});

  /// Called after a receipt is written, so the screens behind can catch up.
  final VoidCallback? onSaved;

  final EntryApi core;

  @override
  State<EntryScreen> createState() => _EntryScreenState();
}

class _EntryScreenState extends State<EntryScreen> {
  final TextEditingController _merchant = TextEditingController();
  final TextEditingController _place = TextEditingController();
  final TextEditingController _note = TextEditingController();
  final List<_RowState> _rows = [_RowState()];

  /// Enter moves on: the field is done, the form is not. The chain runs date →
  /// store → category → amount → the next row, which is the order a receipt is
  /// read in.
  final FocusNode _merchantFocus = FocusNode();
  final FocusNode _noteFocus = FocusNode();

  String _date = today();
  List<CategoryDto> _categories = const [];
  List<String> _stores = const [];
  List<String> _places = const [];
  List<String> _recent = const [];
  Map<int, int> _categoryRates = const {};
  String? _locationName;

  /// The location this form filled in last, to tell it from a user's choice.
  String? _lastDefaultPlace;

  /// The store the context was loaded for, so leaving the field unchanged does
  /// not cost a round trip and a repaint for no news.
  String? _loadedFor;
  bool _loadedOnce = false;

  Timer? _debounce;
  Preview? _preview;
  String? _problem;
  bool _saving = false;
  String? _saved;

  @override
  void initState() {
    super.initState();
    unawaited(_boot());
  }

  @override
  void dispose() {
    _debounce?.cancel();
    _merchant.dispose();
    _place.dispose();
    _note.dispose();
    _merchantFocus.dispose();
    _noteFocus.dispose();
    for (final row in _rows) {
      row.dispose();
    }
    super.dispose();
  }

  Future<void> _boot() async {
    try {
      final results = await (
        widget.core.listCategories(),
        widget.core.listMerchants(),
        widget.core.listLocations(),
        widget.core.listAreas(),
      ).wait;
      final (categories, merchants, locations, areas) = results;
      if (!mounted) return;
      final known = locations.map((l) => l.name).toSet();
      setState(() {
        _categories = categories;
        _stores = [for (final m in merchants) m.name];
        // Places already shopped first, then every other jurisdiction the
        // state publishes. A name from the second list becomes a real location
        // on save, with the published rate and a link that follows the
        // quarterly refresh.
        _places = [
          for (final l in locations) l.name,
          for (final a in areas)
            if (!known.contains(a.name)) a.name,
        ];
      });
      await _loadContext(null);
    } catch (error) {
      if (mounted) setState(() => _problem = '$error');
    }
  }

  /// What the app knows about this store.
  Future<void> _loadContext(String? merchant, {bool force = false}) async {
    // Leaving the store field commits it whether or not a letter changed, so
    // this runs on every tab-out. Reloading for a name we already hold would
    // be a round trip and a repaint for no news.
    if (!force && _loadedOnce && merchant == _loadedFor) return;
    try {
      final context = await widget.core.entryContext(merchant);
      if (!mounted) return;
      setState(() {
        _loadedFor = merchant;
        _loadedOnce = true;
        _locationName = context.locationName;
        _recent = context.recentMerchants;
        _categoryRates = {
          for (final rate in context.categoryRates) rate.categoryId: rate.ratePpm,
        };
        // Filled in only when the user has not chosen one themselves: naming a
        // store must not silently move an expense already placed somewhere.
        if (_place.text.isEmpty || _place.text == _lastDefaultPlace) {
          _place.text = context.locationName ?? '';
        }
        _lastDefaultPlace = context.locationName ?? '';
      });
      // The store's remembered category is deliberately not filled in. A
      // category that appears by itself is one the user did not choose and may
      // not notice, and the receipt in their hand is the authority on what they
      // bought.
    } catch (error) {
      if (mounted) setState(() => _problem = '$error');
    }
  }

  /// The categories nobody else has taken, plus this row's own.
  ///
  /// Filing two rows against one category is not a receipt with two lines, it
  /// is one line entered twice — the splits would be indistinguishable in every
  /// report. Removing the taken ones says so before the user can do it.
  List<CategoryDto> _availableTo(_RowState row) {
    final taken = {
      for (final other in _rows)
        if (!identical(other, row) && other.categoryId != null) other.categoryId!,
    };
    final available = [
      for (final category in _categories)
        if (!taken.contains(category.id)) category,
    ];
    // Alphabetical, not the seed's grouping order. The list is filtered by
    // typing, so its only job unfiltered is to be scannable — and the one order
    // a reader can predict is the alphabet.
    available.sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));
    return available;
  }

  /// The draft as the core wants it.
  SubtotalDraft _draft([List<DraftRow>? overrides]) {
    final rows = <DraftRow>[];
    if (overrides == null) {
      for (final row in _rows) {
        // What the field comes to, which may be several figures added by the
        // core — three items rung up under one category are one subtotal.
        final cents = amountValue(row.amount.text);
        // A row still being typed is left out rather than sent as zero: a
        // $0.00 line would claim the user meant to file nothing under that
        // category.
        if (cents != null) {
          rows.add(DraftRow(categoryId: row.categoryId, subtotal: cents));
        }
      }
    } else {
      rows.addAll(overrides);
    }

    final place = _place.text.trim();
    return SubtotalDraft(
      txnDate: _date,
      merchant: _merchant.text.trim().isEmpty ? null : _merchant.text.trim(),
      location: place.isEmpty ? _locationName : place,
      rows: rows,
      notes: _note.text.trim().isEmpty ? null : _note.text.trim(),
    );
  }

  void _schedulePreview() {
    _debounce?.cancel();
    _debounce = Timer(_settle, () => unawaited(_refreshPreview()));
  }

  Future<void> _refreshPreview() async {
    final draft = _draft();
    if (draft.rows.isEmpty) {
      if (mounted) {
        setState(() {
          _preview = null;
          _problem = null;
        });
      }
      return;
    }
    try {
      final preview = await widget.core.previewSubtotals(draft);
      if (!mounted) return;
      setState(() {
        _preview = preview;
        _problem = null;
      });
    } catch (error) {
      if (!mounted) return;
      setState(() {
        _preview = null;
        _problem = '$error';
      });
    }
  }

  void _addRow() {
    setState(() => _rows.add(_RowState()));
    _schedulePreview();
  }

  void _removeRow(int index) {
    setState(() => _rows.removeAt(index).dispose());
    _schedulePreview();
  }

  Future<void> _save() async {
    if (_saving) return;
    setState(() {
      _saving = true;
      _saved = null;
    });

    try {
      // Nothing is written until the user has seen the numbers and said yes.
      // The dialog re-previews through the core on every correction, so what
      // they approve is exactly what gets saved.
      final preview = await widget.core.previewSubtotals(_draft());
      if (!mounted) return;
      final approved = await showReviewDialog(
        context: context,
        preview: preview,
        recompute: (edits) => widget.core.previewSubtotals(_draft(edits)),
      );
      if (approved == null || !mounted) return;

      final id = await widget.core.saveSubtotals(_draft(approved));
      if (!mounted) return;
      // A blank form, not a re-run of the last one. Leaving the store and the
      // amounts on screen invites a second click that files the same expense
      // twice, and every field that survives is one the next expense has to be
      // checked against.
      setState(() {
        _saved = 'Saved expense #$id.';
        _merchant.clear();
        _place.clear();
        _note.clear();
        _date = today();
        for (final row in _rows) {
          row.dispose();
        }
        _rows
          ..clear()
          ..add(_RowState());
        _preview = null;
        _problem = null;
      });
      // Forced past the "same store, nothing to do" guard, because one thing
      // has changed: this store was just shopped at, so the recent-store chips
      // are out of date by exactly this receipt.
      await _loadContext(null, force: true);
      widget.onSaved?.call();
    } catch (error) {
      if (mounted) setState(() => _problem = '$error');
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final preview = _preview;
    // Blockers first, then warnings: an expense that cannot be saved should say
    // so while it is being typed rather than after the button is pressed.
    final String status = _problem ??
        (preview == null
            ? ''
            : [...preview.blockers, ...preview.warnings].join(' · '));
    final bool blocked = preview == null || !preview.canSave || _problem != null;

    return ListView(
      padding: const EdgeInsets.fromLTRB(16, 16, 16, 96),
      children: [
        _Field(
          label: 'Date',
          child: DateField(
            value: _date,
            // The first thing a receipt needs is its date, and it is already
            // filled in — so the form opens with it selected, ready to be
            // typed over or passed straight through with Enter.
            autofocus: true,
            onSubmitted: _merchantFocus.requestFocus,
            onChanged: (iso) {
              // Empty while the segments are half-typed. Keeping the last good
              // date would preview a receipt against a day the user is in the
              // middle of changing.
              setState(() => _date = iso);
              _schedulePreview();
            },
          ),
        ),
        _Field(
          label: 'Store',
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Combobox(
                controller: _merchant,
                focusNode: _merchantFocus,
                options: _stores,
                hint: 'Select a store',
                emptyLabel: 'No stores yet.',
                onSubmitted: () => _rows.first.categoryFocus.requestFocus(),
                onCommit: (name) async {
                  await _loadContext(name.isEmpty ? null : name);
                  _schedulePreview();
                },
              ),
              if (_recent.isNotEmpty)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Wrap(
                    spacing: 8,
                    runSpacing: 4,
                    children: [
                      for (final name in _recent)
                        ActionChip(
                          label: Text(name),
                          onPressed: () {
                            _merchant.text = name;
                            unawaited(_loadContext(name).then((_) {
                              _schedulePreview();
                            }));
                          },
                        ),
                    ],
                  ),
                ),
            ],
          ),
        ),
        _Field(
          label: 'Categories',
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (int i = 0; i < _rows.length; i++)
                _CategoryRow(
                  // Keyed by the row's own identity, so adding or removing one
                  // never rebuilds the field another row is being typed into:
                  // the caret would go with it.
                  key: ObjectKey(_rows[i]),
                  row: _rows[i],
                  options: _availableTo(_rows[i]),
                  ratePpm: _categoryRates[_rows[i].categoryId],
                  onCategory: (id) {
                    setState(() => _rows[i].categoryId = id);
                    _schedulePreview();
                  },
                  onAmount: _schedulePreview,
                  // Enter through a row: category to its amount, then on to the
                  // next row's category, or to the note when this was the last.
                  onCategoryDone: _rows[i].amountFocus.requestFocus,
                  onAmountDone: () => i + 1 < _rows.length
                      ? _rows[i + 1].categoryFocus.requestFocus()
                      : _noteFocus.requestFocus(),
                  onRemove: _rows.length > 1 ? () => _removeRow(i) : null,
                ),
              Align(
                alignment: Alignment.centerLeft,
                child: Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: FilledButton.tonalIcon(
                    onPressed: _addRow,
                    icon: const Icon(Icons.add),
                    label: const Text('Add category'),
                  ),
                ),
              ),
            ],
          ),
        ),
        _Field(
          label: 'Location',
          child: Combobox(
            controller: _place,
            options: _places,
            hint: 'Start typing a city…',
            emptyLabel: 'Start typing to find a city.',
            // A search rather than a list to browse: there are four hundred
            // jurisdictions, and dropping them over the form on focus helps
            // nobody.
            openOnFocus: false,
            // Only a published jurisdiction, or somewhere already shopped. A
            // name the state does not publish would be created at 0% and
            // record no tax on every receipt filed against it — which is the
            // worst thing this app can do quietly.
            strict: true,
            onCommit: (name) async {
              if (name.isEmpty) {
                _schedulePreview();
                return;
              }
              // Created here rather than on save, because the preview has to be
              // able to name it: the core refuses a location it does not know,
              // and showing the home rate instead would put a wrong number on
              // screen.
              try {
                await widget.core.ensureLocation(name);
              } catch (error) {
                if (mounted) setState(() => _problem = '$error');
              }
              _schedulePreview();
            },
          ),
        ),
        _Field(
          label: 'Note',
          child: TextField(
            controller: _note,
            focusNode: _noteFocus,
            decoration: const InputDecoration(
              hintText: 'optional',
              border: OutlineInputBorder(),
              isDense: true,
            ),
          ),
        ),
        const SizedBox(height: 8),
        Row(
          children: [
            FilledButton(
              onPressed: _saving || blocked ? null : () => unawaited(_save()),
              child: const Text('Save expense'),
            ),
            const SizedBox(width: 16),
            Expanded(
              child: Semantics(
                liveRegion: true,
                child: Text(
                  _saved ?? status,
                  style: theme.textTheme.bodyMedium?.copyWith(
                    color: _problem != null ||
                            (preview?.blockers.isNotEmpty ?? false)
                        ? theme.colorScheme.error
                        : theme.colorScheme.onSurfaceVariant,
                  ),
                ),
              ),
            ),
          ],
        ),
      ],
    );
  }
}

class _Field extends StatelessWidget {
  const _Field({required this.label, required this.child});

  final String label;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.only(bottom: 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.only(bottom: 6),
            child: Text(label, style: theme.textTheme.labelLarge),
          ),
          child,
        ],
      ),
    );
  }
}

class _CategoryRow extends StatelessWidget {
  const _CategoryRow({
    super.key,
    required this.row,
    required this.options,
    required this.ratePpm,
    required this.onCategory,
    required this.onAmount,
    required this.onCategoryDone,
    required this.onAmountDone,
    required this.onRemove,
  });

  final _RowState row;
  final List<CategoryDto> options;
  final int? ratePpm;
  final void Function(int? id) onCategory;
  final VoidCallback onAmount;
  final VoidCallback onCategoryDone;
  final VoidCallback onAmountDone;
  final VoidCallback? onRemove;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Combobox(
                  controller: row.category,
                  focusNode: row.categoryFocus,
                  options: [for (final c in options) c.name],
                  hint: 'Select a category',
                  emptyLabel: 'No categories yet.',
                  onSubmitted: onCategoryDone,
                  onCommit: (name) {
                    final match = options
                        .where((c) => c.name.toLowerCase() == name.toLowerCase())
                        .firstOrNull;
                    onCategory(match?.id);
                  },
                ),
                // The rate belongs beside the category that determines it, and
                // shows as soon as one is picked: "what will this be taxed at"
                // is a question about the category alone.
                if (ratePpm != null)
                  Padding(
                    padding: const EdgeInsets.only(top: 4, left: 4),
                    child: Text(
                      categoryRateLabel(ratePpm!),
                      style: theme.textTheme.bodySmall
                          ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
                    ),
                  ),
              ],
            ),
          ),
          const SizedBox(width: 8),
          SizedBox(
            width: 140,
            child: CentsField(
              controller: row.amount,
              focusNode: row.amountFocus,
              onChanged: (_) => onAmount(),
              onSubmitted: (_) => onAmountDone(),
            ),
          ),
          SizedBox(
            width: 48,
            child: onRemove == null
                ? null
                : IconButton(
                    onPressed: onRemove,
                    icon: const Icon(Icons.close),
                    iconSize: 20,
                    tooltip: 'Remove this category',
                  ),
          ),
        ],
      ),
    );
  }
}

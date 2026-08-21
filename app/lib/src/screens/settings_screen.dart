import 'dart:io';
import 'dart:typed_data' show Uint8List;

import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';

import '../api.dart';
import 'home_area_form.dart';

typedef SyncStatusLoader = Future<SyncStatus> Function();
typedef BundleExporter = Future<Uint8List> Function({
  String? peer,
  required String phrase,
});
typedef DeliveryMarker = Future<void> Function({
  String? peer,
  required String phrase,
  required Uint8List bytes,
});
typedef BundleImporter = Future<Applied> Function({
  required Uint8List bytes,
  required String phrase,
});
typedef BundleWriter = Future<void> Function(String path, Uint8List bytes);
typedef BundleReader = Future<Uint8List> Function(String path);
typedef DirectoryFinder = Future<String> Function();

/// Settings: the handful of answers the app asked for once and then relied on.
///
/// The home area was asked at first run and there was no way back to it, so a
/// user who moved, or who picked the wrong town in a list of four hundred, had
/// no way to correct the rate every later expense was estimated with.
///
/// Sync lives here too rather than on a destination of its own: it is a thing
/// you set up once and touch when you move a file, not somewhere you work.
/// The whole page scrolls as one — the form is embedded rather than scrolling
/// inside a scroller, because a section you cannot reach is a section that
/// does not exist.
class SettingsScreen extends StatelessWidget {
  const SettingsScreen({super.key, this.databasePath});

  final String? databasePath;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return ListView(
      children: [
        HomeAreaForm(
          submitLabel: 'Save location',
          onSaved: () {},
          embedded: true,
        ),
        const Divider(height: 32),
        const Padding(
          padding: EdgeInsets.fromLTRB(16, 0, 16, 16),
          child: SyncSection(),
        ),
        if (databasePath != null) ...[
          const Divider(height: 32),
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text('Database', style: theme.textTheme.labelMedium),
                const SizedBox(height: 4),
                SelectableText(
                  databasePath!,
                  style: theme.textTheme.bodySmall
                      ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
                ),
              ],
            ),
          ),
        ],
      ],
    );
  }
}

/// Exchanging changes with another device, through a file.
///
/// # Why this is two buttons and not one
///
/// There is no server and no account. Each device writes what the other has
/// not seen into one sealed file; how that file travels — a synced folder, a
/// stick, a cable — is deliberately not the app's business, which is exactly
/// what makes all three work. So the app can offer "write one" and "read one",
/// and nothing in between.
///
/// # The phrase is the whole defence
///
/// A bundle is a household's entire ledger, left somewhere a second device can
/// reach it. It is encrypted with the pairing phrase and with nothing else, so
/// the phrase is obscured on screen, is never stored, and is never repeated
/// back in a message or an error — errors get pasted into bug reports.
///
/// # Why the file path is typed
///
/// A system file picker (`file_selector`) would be better and is the right
/// answer on mobile, where the destination genuinely comes from a document
/// picker. This is a text field for now: no new dependency, and a full path is
/// something a desktop user can paste.
class SyncSection extends StatefulWidget {
  const SyncSection({
    super.key,
    this.loadStatus = syncStatus,
    this.export = exportBundle,
    this.markSent = markDelivered,
    this.import = importBundle,
    this.writeFile = writeNewFile,
    this.readFile = readFileBytes,
    this.defaultDirectory = documentsDirectory,
  });

  /// Every call this section makes is injectable, and defaults to the real
  /// API. A widget test has no bridge behind it, so a screen that reached for
  /// the generated bindings directly could only be tested by starting Rust.
  final SyncStatusLoader loadStatus;
  final BundleExporter export;
  final DeliveryMarker markSent;
  final BundleImporter import;
  final BundleWriter writeFile;
  final BundleReader readFile;
  final DirectoryFinder defaultDirectory;

  @override
  State<SyncSection> createState() => _SyncSectionState();
}

class _SyncSectionState extends State<SyncSection> {
  final TextEditingController _exportPhrase = TextEditingController();
  final TextEditingController _exportPath = TextEditingController();
  final TextEditingController _importPhrase = TextEditingController();
  final TextEditingController _importPath = TextEditingController();

  SyncStatus? _status;
  String? _statusProblem;

  /// The device this file is for, when there is more than one to choose from.
  /// Left null with one peer or none: the core resolves that case itself, and
  /// with several it refuses rather than send one device's history to another.
  String? _peer;

  bool _showExportPhrase = false;
  bool _showImportPhrase = false;
  bool _exporting = false;
  bool _importing = false;

  String? _exportNote;
  bool _exportFailed = false;
  String? _importNote;
  bool _importFailed = false;
  Applied? _imported;

  @override
  void initState() {
    super.initState();
    _load();
    _fillInDefaultPath();
  }

  @override
  void dispose() {
    _exportPhrase.dispose();
    _exportPath.dispose();
    _importPhrase.dispose();
    _importPath.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    try {
      final status = await widget.loadStatus();
      if (!mounted) return;
      setState(() {
        _status = status;
        _statusProblem = null;
        // A peer that has gone away — or a second one that has appeared —
        // must not leave a stale name selected, since that name is what the
        // watermark would be written against.
        if (_peer != null && !status.peers.contains(_peer)) _peer = null;
      });
    } catch (error) {
      if (mounted) setState(() => _statusProblem = '$error');
    }
  }

  /// Offer somewhere to write, so the first export is one field rather than
  /// two. Only ever a suggestion: the user can put the file anywhere, and on a
  /// phone the real answer is a document picker.
  Future<void> _fillInDefaultPath() async {
    try {
      final dir = await widget.defaultDirectory();
      if (!mounted) return;
      final suggestion = '$dir${Platform.pathSeparator}${defaultFileName()}';
      setState(() {
        if (_exportPath.text.isEmpty) _exportPath.text = suggestion;
      });
    } catch (_) {
      // A platform that cannot name a documents directory is not a failure
      // worth a message: the field simply starts empty and is typed into.
    }
  }

  /// Write everything the peer has not been sent, then record it as sent.
  ///
  /// The order is the point. The core deliberately does not advance the
  /// watermark when it builds a bundle, because a file that was written but
  /// never reached the other device must not count as delivered — everything
  /// in it would then never be sent again. From this side, writing the file
  /// *is* the delivery: the transport is a folder or a stick whose far end the
  /// app cannot see, so the last moment it can honestly vouch for is the
  /// moment the bytes are on disk.
  Future<void> _doExport() async {
    final phrase = _exportPhrase.text;
    final path = _exportPath.text.trim();
    if (phrase.isEmpty) {
      setState(() {
        _exportFailed = true;
        _exportNote = 'Enter the pairing phrase. It is the same phrase on both '
            'devices, and it is the only thing protecting the file on the way '
            'over.';
      });
      return;
    }
    if (path.isEmpty) {
      setState(() {
        _exportFailed = true;
        _exportNote = 'Say where to write the file.';
      });
      return;
    }
    final status = _status;
    if (status != null && status.peers.length > 1 && _peer == null) {
      setState(() {
        _exportFailed = true;
        _exportNote = 'This database syncs with more than one device. Choose '
            'which one this file is for — guessing would send one device’s '
            'history to another.';
      });
      return;
    }

    setState(() {
      _exporting = true;
      _exportNote = null;
      _exportFailed = false;
    });
    try {
      final bytes = await widget.export(peer: _peer, phrase: phrase);
      await widget.writeFile(path, bytes);

      final paired = status?.isPaired ?? true;
      final addressee = _peer ?? (status?.peers.isNotEmpty ?? false ? status!.peers.first : null);
      String note = 'Wrote $path (${formatBytes(bytes.length)})'
          '${addressee == null ? '' : ' for $addressee'}.';
      if (paired) {
        try {
          // Only now, and only for a peer that exists: the file is on disk, so
          // from here it is the other device's to read.
          await widget.markSent(peer: _peer, phrase: phrase, bytes: bytes);
          note = '$note Marked as sent — the next file carries only what '
              'changes from now on. Move it however you like; it is sealed '
              'with the phrase, so it protects itself on the way.';
        } catch (error) {
          // The bytes are on disk either way, which is the part that matters.
          // Being unable to record it costs a re-send, not data.
          note = '$note Could not record it as sent, so the next file will '
              'carry this history again: $error';
        }
      } else {
        note = '$note This is the first file of a pairing: nothing here knows '
            'the other device yet, so nothing was recorded as sent and the '
            'next export carries the same history again. Import that device’s '
            'reply and it settles.';
      }
      if (!mounted) return;
      setState(() {
        _exportNote = note;
        _exportFailed = false;
      });
      await _load();
    } catch (error) {
      // Never the phrase, here or anywhere: an error message is the last place
      // a secret should turn up.
      if (mounted) {
        setState(() {
          _exportFailed = true;
          _exportNote = 'Could not write the file: $error';
        });
      }
    } finally {
      if (mounted) setState(() => _exporting = false);
    }
  }

  Future<void> _doImport() async {
    final phrase = _importPhrase.text;
    final path = _importPath.text.trim();
    if (phrase.isEmpty) {
      setState(() {
        _importFailed = true;
        _importNote = 'Enter the pairing phrase the file was sealed with — the '
            'same one the other device used.';
      });
      return;
    }
    if (path.isEmpty) {
      setState(() {
        _importFailed = true;
        _importNote = 'Say which file to read.';
      });
      return;
    }

    setState(() {
      _importing = true;
      _importNote = null;
      _importFailed = false;
      _imported = null;
    });
    try {
      final bytes = await widget.readFile(path);
      final applied = await widget.import(bytes: bytes, phrase: phrase);
      if (!mounted) return;
      setState(() {
        _imported = applied;
        _importNote = 'Read $path.';
        _importFailed = false;
      });
      // Conflicts recorded by this import are waiting for a human, and the
      // sending device is now a peer even if it was not one a moment ago.
      await _load();
    } catch (error) {
      if (mounted) {
        setState(() {
          _importFailed = true;
          _importNote = '$error';
        });
      }
    } finally {
      if (mounted) setState(() => _importing = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final SyncStatus? status = _status;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text('Sync', style: theme.textTheme.titleMedium),
        const SizedBox(height: 8),
        Text(
          'Two devices keep up with each other by passing a file. There is no '
          'server and no account: each device writes what the other has not '
          'seen, and the file travels however you like — a synced folder, a '
          'stick, a cable.',
          style: theme.textTheme.bodyMedium
              ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
        ),
        const SizedBox(height: 16),
        if (_statusProblem != null)
          Text(
            'Could not read the sync status: $_statusProblem',
            style: theme.textTheme.bodyMedium
                ?.copyWith(color: theme.colorScheme.error),
          ),
        if (status != null) ..._identity(theme, status),
        const SizedBox(height: 24),
        Text('Export', style: theme.textTheme.titleSmall),
        const SizedBox(height: 4),
        Text(
          'Writes everything the other device has not been sent into one '
          'sealed file.',
          style: theme.textTheme.bodySmall
              ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
        ),
        const SizedBox(height: 12),
        if (status != null && status.peers.length > 1) ...[
          DropdownMenu<String>(
            initialSelection: _peer,
            label: const Text('Send to'),
            helperText: 'Which device this file is for.',
            onSelected: (value) => setState(() => _peer = value),
            dropdownMenuEntries: [
              for (final peer in status.peers)
                DropdownMenuEntry<String>(value: peer, label: peer),
            ],
          ),
          const SizedBox(height: 12),
        ],
        _phraseField(
          // Keyed because the two phrase boxes are deliberately labelled the
          // same — it is the same phrase — and a test has to be able to say
          // which one it means.
          fieldKey: const Key('export-phrase'),
          controller: _exportPhrase,
          label: 'Pairing phrase',
          shown: _showExportPhrase,
          onToggle: () =>
              setState(() => _showExportPhrase = !_showExportPhrase),
          helper: 'The same phrase on both devices. It is what the file is '
              'encrypted with, and it is not stored anywhere — nothing can '
              'read a file whose phrase is lost.',
        ),
        const SizedBox(height: 12),
        TextField(
          key: const Key('export-path'),
          controller: _exportPath,
          decoration: const InputDecoration(
            labelText: 'Write to',
            helperText: 'The full path of the file to write. It must not '
                'already exist.',
            border: OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 12),
        Row(
          children: [
            FilledButton(
              onPressed: _exporting ? null : () => _doExport(),
              child: Text(_exporting ? 'Writing…' : 'Export'),
            ),
          ],
        ),
        if (_exportNote != null) ...[
          const SizedBox(height: 8),
          Semantics(
            liveRegion: true,
            child: Text(
              _exportNote!,
              style: theme.textTheme.bodyMedium?.copyWith(
                color: _exportFailed
                    ? theme.colorScheme.error
                    : theme.colorScheme.onSurfaceVariant,
              ),
            ),
          ),
        ],
        const SizedBox(height: 24),
        Text('Import', style: theme.textTheme.titleSmall),
        const SizedBox(height: 4),
        Text(
          'Applies a file the other device wrote.',
          style: theme.textTheme.bodySmall
              ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
        ),
        const SizedBox(height: 12),
        _phraseField(
          fieldKey: const Key('import-phrase'),
          controller: _importPhrase,
          label: 'Pairing phrase',
          shown: _showImportPhrase,
          onToggle: () =>
              setState(() => _showImportPhrase = !_showImportPhrase),
          helper: 'The phrase the file was sealed with.',
        ),
        const SizedBox(height: 12),
        TextField(
          key: const Key('import-path'),
          controller: _importPath,
          decoration: const InputDecoration(
            labelText: 'Read from',
            helperText: 'The full path of the file to apply.',
            border: OutlineInputBorder(),
          ),
        ),
        const SizedBox(height: 12),
        Row(
          children: [
            FilledButton.tonal(
              onPressed: _importing ? null : () => _doImport(),
              child: Text(_importing ? 'Reading…' : 'Import'),
            ),
          ],
        ),
        if (_importNote != null) ...[
          const SizedBox(height: 8),
          Semantics(
            liveRegion: true,
            child: Text(
              _importNote!,
              style: theme.textTheme.bodyMedium?.copyWith(
                color: _importFailed
                    ? theme.colorScheme.error
                    : theme.colorScheme.onSurfaceVariant,
              ),
            ),
          ),
        ],
        if (_imported != null) ..._outcome(theme, _imported!),
      ],
    );
  }

  /// Who this device is, and who it syncs with.
  List<Widget> _identity(ThemeData theme, SyncStatus status) {
    final quiet =
        theme.textTheme.bodySmall?.copyWith(color: theme.colorScheme.onSurfaceVariant);
    return [
      Text('This device', style: theme.textTheme.labelMedium),
      const SizedBox(height: 4),
      SelectableText(status.thisDevice, style: theme.textTheme.bodyMedium),
      const SizedBox(height: 4),
      Text(
        'The name of this database, not of the machine. A backup restored onto '
        'new hardware carries on as this same device.',
        style: quiet,
      ),
      const SizedBox(height: 16),
      Text('Syncs with', style: theme.textTheme.labelMedium),
      const SizedBox(height: 4),
      if (status.peers.isEmpty)
        Text(
          'No other device yet. Export a file and import it there to start a '
          'pairing.',
          style: quiet,
        )
      else
        for (final peer in status.peers)
          SelectableText(peer, style: theme.textTheme.bodyMedium),
      if (status.openConflicts > 0) ...[
        const SizedBox(height: 16),
        Card(
          margin: EdgeInsets.zero,
          color: theme.colorScheme.tertiaryContainer,
          child: Padding(
            padding: const EdgeInsets.all(12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  '${status.openConflicts} '
                  '${status.openConflicts == 1 ? 'disagreement' : 'disagreements'} '
                  'waiting to be looked at',
                  style: theme.textTheme.titleSmall
                      ?.copyWith(color: theme.colorScheme.onTertiaryContainer),
                ),
                const SizedBox(height: 4),
                Text(
                  'Both devices changed the same thing. The later edit is in '
                  'place and the earlier one was kept rather than dropped — '
                  'losing a disagreement does not mean that version was wrong. '
                  'The receipts involved are flagged in the Inbox.',
                  style: theme.textTheme.bodySmall
                      ?.copyWith(color: theme.colorScheme.onTertiaryContainer),
                ),
              ],
            ),
          ),
        ),
      ],
    ];
  }

  /// What the last import did.
  ///
  /// The three counts are the headline. Deferred changes and unknown kinds are
  /// printed in full underneath rather than as a number, because both mean
  /// *data did not arrive* and a count reads exactly like a clean import.
  List<Widget> _outcome(ThemeData theme, Applied applied) {
    final quiet =
        theme.textTheme.bodySmall?.copyWith(color: theme.colorScheme.onSurfaceVariant);
    return [
      const SizedBox(height: 12),
      Text('Applied ${applied.applied}', style: theme.textTheme.bodyMedium),
      Text('Changes taken, because they were newer than anything here.',
          style: quiet),
      const SizedBox(height: 8),
      Text('Ignored ${applied.ignored}', style: theme.textTheme.bodyMedium),
      Text('This device already had the same or newer. Nothing was lost.',
          style: quiet),
      const SizedBox(height: 8),
      Text('Conflicts ${applied.conflicts}', style: theme.textTheme.bodyMedium),
      Text(
        applied.conflicts == 0
            ? 'Nothing was edited on both devices at once.'
            : 'Both devices had changed the same thing. The later edit is in '
                'place and the earlier one was kept rather than dropped — '
                'losing a conflict does not mean that version was wrong. The '
                'receipts involved are back in the Inbox.',
        style: quiet,
      ),
      const SizedBox(height: 8),
      if (applied.deferred.isEmpty)
        Text('Deferred 0', style: theme.textTheme.bodyMedium)
      else
        Card(
          margin: EdgeInsets.zero,
          color: theme.colorScheme.tertiaryContainer,
          child: Padding(
            padding: const EdgeInsets.all(12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'Deferred ${applied.deferred.length} — these did not arrive',
                  style: theme.textTheme.titleSmall
                      ?.copyWith(color: theme.colorScheme.onTertiaryContainer),
                ),
                const SizedBox(height: 4),
                Text(
                  'Each one points at something that is not on this device, '
                  'usually because an earlier file was never imported. Import '
                  'that file if you still have it; otherwise export again from '
                  'the other device once it has read this one’s reply.',
                  style: theme.textTheme.bodySmall
                      ?.copyWith(color: theme.colorScheme.onTertiaryContainer),
                ),
                const SizedBox(height: 8),
                for (final line in applied.deferred)
                  Text(
                    line,
                    style: theme.textTheme.bodySmall
                        ?.copyWith(color: theme.colorScheme.onTertiaryContainer),
                  ),
              ],
            ),
          ),
        ),
      if (applied.unknownKinds.isNotEmpty) ...[
        const SizedBox(height: 8),
        Card(
          margin: EdgeInsets.zero,
          color: theme.colorScheme.tertiaryContainer,
          child: Padding(
            padding: const EdgeInsets.all(12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'This version does not understand: '
                  '${applied.unknownKinds.join(', ')}',
                  style: theme.textTheme.titleSmall
                      ?.copyWith(color: theme.colorScheme.onTertiaryContainer),
                ),
                const SizedBox(height: 4),
                Text(
                  'They were written by a newer release. Nothing was lost on '
                  'the device that sent them — update this one and import the '
                  'file again.',
                  style: theme.textTheme.bodySmall
                      ?.copyWith(color: theme.colorScheme.onTertiaryContainer),
                ),
              ],
            ),
          ),
        ),
      ],
    ];
  }

  /// A phrase box, obscured, with a way to check what was typed.
  ///
  /// Obscured because the file it protects is a whole ledger and somebody is
  /// often standing next to the screen while a pairing is set up. Revealable
  /// because it is typed from memory on two devices, and an unreadable field
  /// with no way to check it turns one typo into "the phrase does not match"
  /// on the other machine an hour later.
  Widget _phraseField({
    required Key fieldKey,
    required TextEditingController controller,
    required String label,
    required bool shown,
    required VoidCallback onToggle,
    required String helper,
  }) {
    return TextField(
      key: fieldKey,
      controller: controller,
      obscureText: !shown,
      autocorrect: false,
      enableSuggestions: false,
      decoration: InputDecoration(
        labelText: label,
        helperText: helper,
        helperMaxLines: 4,
        border: const OutlineInputBorder(),
        suffixIcon: IconButton(
          icon: Icon(shown ? Icons.visibility_off : Icons.visibility),
          tooltip: shown ? 'Hide the phrase' : 'Show the phrase',
          onPressed: onToggle,
        ),
      ),
    );
  }
}

/// A size a person can read, for the "wrote it" message.
String formatBytes(int bytes) {
  if (bytes < 1024) return '$bytes bytes';
  final double kb = bytes / 1024;
  if (kb < 1024) return '${kb.toStringAsFixed(1)} kB';
  return '${(kb / 1024).toStringAsFixed(1)} MB';
}

/// A name for a file that does not exist yet.
///
/// Stamped with the local date and time because a sync file is never
/// overwritten, so a fixed name would refuse the second export of the day and
/// leave the user renaming things by hand.
String defaultFileName([DateTime? now]) {
  final when = now ?? DateTime.now();
  String two(int n) => n.toString().padLeft(2, '0');
  return 'pacc-${when.year}${two(when.month)}${two(when.day)}'
      '-${two(when.hour)}${two(when.minute)}.paccsync';
}

/// Where to suggest writing a bundle.
///
/// `path_provider`, not a guess: only the platform knows where a user's
/// documents are, and on Android the answer is not a path any rule could
/// derive.
Future<String> documentsDirectory() async =>
    (await getApplicationDocumentsDirectory()).path;

/// Write a bundle that did not exist a moment ago.
///
/// Refuses to overwrite, exactly as `pacc sync export` does: a file you cannot
/// get back is data until its owner says otherwise, and the one thing a stale
/// sync file looks like is a fresh one. `dart:io` has no exclusive-create
/// mode, so the check is a courtesy rather than a lock — two exports racing
/// for one name is not a case worth more machinery than that.
///
/// `flush: true` because the caller marks the bundle delivered on the strength
/// of this returning, and "the bytes are in the page cache" is not the same
/// promise as "the bytes are on the stick you are about to pull out".
Future<void> writeNewFile(String path, Uint8List bytes) async {
  final file = File(path);
  if (await file.exists()) {
    throw '$path already exists, and a sync file is never overwritten. '
        'Give this one another name, or delete that one first.';
  }
  await file.parent.create(recursive: true);
  await file.writeAsBytes(bytes, flush: true);
}

/// Read a bundle off disk.
Future<Uint8List> readFileBytes(String path) => File(path).readAsBytes();

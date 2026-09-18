import 'dart:typed_data' show Uint8List;

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';

import '../api.dart';
import '../browser/browser.dart';
import '../sync/lan_sync.dart';
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
typedef FileDownloader = Future<void> Function(String name, Uint8List bytes);
typedef WholeExporter = Future<Uint8List> Function({required String phrase});
typedef RefusalLoader = Future<String?> Function();

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
  const SettingsScreen({
    super.key,
    this.databasePath,
    this.lan,
    this.browser = kIsWeb,
  });

  final String? databasePath;

  /// Whether the app is a page in a browser, where everything lives in the
  /// browser's storage and nowhere else. Injectable so a test can see the
  /// page as a browser shows it.
  final bool browser;

  /// The running local-network service, when there is one. Null in a test that
  /// is only interested in the rest of the page: the app itself always has one,
  /// because the switch that turns it on has to have something to turn on.
  final LanSync? lan;

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
        if (browser)
          const Padding(
            padding: EdgeInsets.fromLTRB(16, 0, 16, 16),
            child: BrowserStorageNote(),
          ),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
          child: SyncSection(
            browser: browser,
            onStandingChanged: lan?.recheckStanding,
          ),
        ),
        if (lan != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
            child: LanSyncSection(lan: lan!),
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

/// What a browser does with the data, said once where sync is set up.
///
/// A page's storage is the browser's to keep or throw away. Safari clears a
/// site's storage after seven days without a visit, whatever the site asked
/// for; every browser clears it when somebody clears their browsing data; and
/// a private window forgets it on closing. None of that asks the app first.
/// So the page says so plainly, beside the one thing that protects against
/// it: another device holding a copy, kept current with the sync file below.
///
/// Deliberately a note and not a nag. It does not count days or interrupt
/// anything; the people it is for need to read it once.
class BrowserStorageNote extends StatelessWidget {
  const BrowserStorageNote({super.key});

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      margin: EdgeInsets.zero,
      color: theme.colorScheme.tertiaryContainer,
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              'Kept in this browser only',
              style: theme.textTheme.titleSmall
                  ?.copyWith(color: theme.colorScheme.onTertiaryContainer),
            ),
            const SizedBox(height: 4),
            Text(
              'Everything you enter is stored in this browser, on this device, '
              'and nowhere else. Browsers can clear it without asking — Safari '
              'does after seven days without a visit, and clearing browsing '
              'data does it anywhere. Keep another device up to date by '
              'exporting a sync file to it regularly, so your records are '
              'never only here.',
              style: theme.textTheme.bodySmall
                  ?.copyWith(color: theme.colorScheme.onTertiaryContainer),
            ),
          ],
        ),
      ),
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
///
/// A browser is the exception, because a page cannot write to a path or read
/// from one at all. There, exporting is a download and importing is the
/// browser's own file picker — the only two ways a file crosses into or out
/// of a page.
///
/// # Joining, and the file it needs
///
/// A device that has only just been opened has seeded its own chart of
/// accounts under uuids of its own, so importing another device's file would
/// put two of every category side by side. It *joins* instead: throws its seed
/// away and takes the other device's copy. Over Wi-Fi that is the Join button
/// in the section below; a browser has no Wi-Fi sync, so for it a file is the
/// only way, and this section offers the same step with a file.
///
/// The file has to be the right one. What Export writes on a paired device is
/// only what its peer has not been sent, and a join keeps nothing else, so the
/// other device writes a copy of everything for this instead — "For a new
/// device". The core refuses the wrong file before anything is thrown away,
/// and this section passes on why.
class SyncSection extends StatefulWidget {
  const SyncSection({
    super.key,
    this.loadStatus = syncStatus,
    this.export = exportBundle,
    this.exportWhole = exportForNewDevice,
    this.markSent = markDelivered,
    this.import = importBundle,
    this.join = joinBundle,
    this.loadRefusal = joinRefusal,
    this.writeFile = writeNewFile,
    this.readFile = readFileBytes,
    this.defaultDirectory = documentsDirectory,
    this.browser = kIsWeb,
    this.download = downloadFile,
    this.pick = pickFile,
    this.onStandingChanged,
  });

  /// Every call this section makes is injectable, and defaults to the real
  /// API. A widget test has no bridge behind it, so a screen that reached for
  /// the generated bindings directly could only be tested by starting Rust.
  final SyncStatusLoader loadStatus;
  final BundleExporter export;
  final WholeExporter exportWhole;
  final DeliveryMarker markSent;
  final BundleImporter import;
  final BundleImporter join;
  final RefusalLoader loadRefusal;
  final BundleWriter writeFile;
  final BundleReader readFile;
  final DirectoryFinder defaultDirectory;

  /// Download and pick rather than write and read paths. The last two are
  /// used only when this is set, and the three before them only when it is
  /// not.
  final bool browser;
  final FileDownloader download;
  final FilePicker pick;

  /// Called once a file has changed who this device syncs with — a join, or
  /// the import that pairs it — so the Wi-Fi service, which reads that only
  /// after its own exchanges, stops treating the device as unpaired.
  final Future<void> Function()? onStandingChanged;

  @override
  State<SyncSection> createState() => _SyncSectionState();
}

/// A downloaded bundle that nobody has yet said was kept.
typedef _Unconfirmed = ({String? peer, String phrase, Uint8List bytes});

class _SyncSectionState extends State<SyncSection> {
  final TextEditingController _exportPhrase = TextEditingController();
  final TextEditingController _exportPath = TextEditingController();
  final TextEditingController _importPhrase = TextEditingController();
  final TextEditingController _importPath = TextEditingController();
  final TextEditingController _joinPhrase = TextEditingController();
  final TextEditingController _joinPath = TextEditingController();

  SyncStatus? _status;
  String? _statusProblem;

  /// Why this device cannot join another, in the core's words; null when it
  /// can, or when it is paired and the question does not arise.
  String? _refusal;

  /// The device this file is for, when there is more than one to choose from.
  /// Left null with one peer or none: the core resolves that case itself, and
  /// with several it refuses rather than send one device's history to another.
  String? _peer;

  bool _showExportPhrase = false;
  bool _showImportPhrase = false;
  bool _showJoinPhrase = false;
  bool _exporting = false;
  bool _exportingWhole = false;
  bool _importing = false;
  bool _joining = false;

  String? _exportNote;
  bool _exportFailed = false;
  String? _importNote;
  bool _importFailed = false;
  Applied? _imported;
  String? _joinNote;
  bool _joinFailed = false;
  Applied? _joined;

  /// In a browser, the bundle last downloaded for a peer, until somebody says
  /// it was saved. See [_doExport] for why the page cannot tell.
  _Unconfirmed? _unconfirmed;
  bool _marking = false;

  @override
  void initState() {
    super.initState();
    _load();
    // A page has no directory to suggest: its exports are downloads, and the
    // browser decides where those land.
    if (!widget.browser) _fillInDefaultPath();
  }

  @override
  void dispose() {
    _exportPhrase.dispose();
    _exportPath.dispose();
    _importPhrase.dispose();
    _importPath.dispose();
    _joinPhrase.dispose();
    _joinPath.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    try {
      final status = await widget.loadStatus();
      // Asked only of a device that is not paired: a paired one is not
      // offered a join at all, and the core's answer for it would only say so.
      String? refusal;
      if (!status.isPaired) {
        try {
          refusal = await widget.loadRefusal();
        } catch (error) {
          // Unanswerable is not the same as allowed. Joining throws this
          // device's records away, so the button stays off and says why.
          refusal = 'could not check whether this device has records of its '
              'own: $error';
        }
      }
      if (!mounted) return;
      setState(() {
        _status = status;
        _statusProblem = null;
        _refusal = refusal;
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
      final suggestion = '$dir$pathSeparator${defaultFileName()}';
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
  ///
  /// A browser cannot vouch even for that. A download is handed to the
  /// browser, which may save it, ask where to save it, or be told "cancel" —
  /// and tells the page none of it. Recording it as sent on the strength of
  /// the click would mean a cancelled download's contents never reach the
  /// other device. So in a browser the bundle is held until the user says it
  /// was kept, and only then marked; not saying so costs a re-send, not data.
  Future<void> _doExport() async {
    final phrase = _exportPhrase.text;
    final path = widget.browser ? '' : _exportPath.text.trim();
    if (phrase.isEmpty) {
      setState(() {
        _exportFailed = true;
        _exportNote = 'Enter the pairing phrase. It is the same phrase on both '
            'devices, and it is the only thing protecting the file on the way '
            'over.';
      });
      return;
    }
    if (!widget.browser && path.isEmpty) {
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
      _unconfirmed = null;
    });
    try {
      final bytes = await widget.export(peer: _peer, phrase: phrase);
      final String written;
      if (widget.browser) {
        written = defaultFileName();
        await widget.download(written, bytes);
      } else {
        written = path;
        await widget.writeFile(path, bytes);
      }

      final paired = status?.isPaired ?? true;
      final addressee = _peer ?? (status?.peers.isNotEmpty ?? false ? status!.peers.first : null);
      String note = '${widget.browser ? 'Downloaded' : 'Wrote'} $written '
          '(${formatBytes(bytes.length)})'
          '${addressee == null ? '' : ' for $addressee'}.';
      if (paired && widget.browser) {
        _unconfirmed = (peer: _peer, phrase: phrase, bytes: bytes);
        note = '$note Your browser decides where downloads go and does not '
            'say whether this one was kept. Once it is saved, mark it as sent '
            'and the next file carries only what changes from now on; until '
            'then, the next file carries this history again.';
      } else if (paired) {
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

  /// The user says the download was kept, which is the moment a browser
  /// export becomes a delivery.
  Future<void> _confirmSent() async {
    final sent = _unconfirmed;
    if (sent == null) return;
    setState(() => _marking = true);
    try {
      await widget.markSent(
        peer: sent.peer,
        phrase: sent.phrase,
        bytes: sent.bytes,
      );
      if (!mounted) return;
      setState(() {
        _unconfirmed = null;
        _exportFailed = false;
        _exportNote = 'Marked as sent — the next file carries only what '
            'changes from now on. Move it however you like; it is sealed with '
            'the phrase, so it protects itself on the way.';
      });
      await _load();
    } catch (error) {
      if (mounted) {
        setState(() {
          _exportFailed = true;
          _exportNote = 'Could not record it as sent, so the next file will '
              'carry this history again: $error';
        });
      }
    } finally {
      if (mounted) setState(() => _marking = false);
    }
  }

  Future<void> _doImport() async {
    final phrase = _importPhrase.text;
    final path = widget.browser ? '' : _importPath.text.trim();
    if (phrase.isEmpty) {
      setState(() {
        _importFailed = true;
        _importNote = 'Enter the pairing phrase the file was sealed with — the '
            'same one the other device used.';
      });
      return;
    }
    if (!widget.browser && path.isEmpty) {
      setState(() {
        _importFailed = true;
        _importNote = 'Say which file to read.';
      });
      return;
    }

    // Before anything is awaited: a browser opens its picker only from a
    // gesture it can still see, and one `await` is enough to lose it.
    final Future<PickedFile?>? picking =
        widget.browser ? widget.pick() : null;
    PickedFile? picked;
    if (picking != null) {
      try {
        picked = await picking;
      } catch (error) {
        if (mounted) {
          setState(() {
            _importFailed = true;
            _importNote = 'Could not read the file: $error';
          });
        }
        return;
      }
      // Closing the picker is a change of mind, not a failure.
      if (picked == null || !mounted) return;
    }

    setState(() {
      _importing = true;
      _importNote = null;
      _importFailed = false;
      _imported = null;
    });
    try {
      final bytes = picked?.bytes ?? await widget.readFile(path);
      final applied = await widget.import(bytes: bytes, phrase: phrase);
      if (!mounted) return;
      setState(() {
        _imported = applied;
        _importNote = 'Read ${picked?.name ?? path}.';
        _importFailed = false;
      });
      // Conflicts recorded by this import are waiting for a human, and the
      // sending device is now a peer even if it was not one a moment ago.
      await _load();
      await widget.onStandingChanged?.call();
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

  /// Write a copy of everything here, for a device that will join from it.
  ///
  /// Shares the export's phrase and path: it is the same phrase, and a second
  /// pair of boxes for the same two answers would be two places to mistype
  /// them. Never marked as sent — there is nobody to mark it against until
  /// the new device's first file comes back, and that file acknowledges what
  /// this one carried.
  Future<void> _doExportWhole() async {
    final phrase = _exportPhrase.text;
    final path = widget.browser ? '' : _exportPath.text.trim();
    if (phrase.isEmpty) {
      setState(() {
        _exportFailed = true;
        _exportNote = 'Enter the pairing phrase. The new device needs the same '
            'phrase to open the file, and it is the only thing protecting it '
            'on the way over.';
      });
      return;
    }
    if (!widget.browser && path.isEmpty) {
      setState(() {
        _exportFailed = true;
        _exportNote = 'Say where to write the file.';
      });
      return;
    }

    setState(() {
      _exportingWhole = true;
      _exportNote = null;
      _exportFailed = false;
      _unconfirmed = null;
    });
    try {
      final bytes = await widget.exportWhole(phrase: phrase);
      final String written;
      if (widget.browser) {
        written = newDeviceFileName();
        await widget.download(written, bytes);
      } else {
        written = path;
        await widget.writeFile(path, bytes);
      }
      if (!mounted) return;
      setState(() {
        _exportFailed = false;
        _exportNote = '${widget.browser ? 'Downloaded' : 'Wrote'} $written '
            '(${formatBytes(bytes.length)}), a copy of everything on this '
            'device. On the new device, choose Join with a file and pick it. '
            'Nothing here was marked as sent: the new device introduces itself '
            'with its first file back.';
      });
    } catch (error) {
      if (mounted) {
        setState(() {
          _exportFailed = true;
          _exportNote = 'Could not write the file: $error';
        });
      }
    } finally {
      if (mounted) setState(() => _exportingWhole = false);
    }
  }

  /// Take another device's records from a file, replacing everything here.
  ///
  /// The file is chosen before the question is asked, not after: a browser
  /// opens its picker only from a gesture it can still see, and a dialog in
  /// between is enough to lose it. Reading a file changes nothing, so asking
  /// second costs nothing — and it lets the question name the file.
  Future<void> _doJoin(BuildContext context) async {
    final phrase = _joinPhrase.text;
    final path = widget.browser ? '' : _joinPath.text.trim();
    if (phrase.isEmpty) {
      setState(() {
        _joinFailed = true;
        _joinNote = 'Enter the pairing phrase the file was sealed with — the '
            'same one the other device used.';
      });
      return;
    }
    if (!widget.browser && path.isEmpty) {
      setState(() {
        _joinFailed = true;
        _joinNote = 'Say which file to join from.';
      });
      return;
    }

    // Before anything is awaited, for the reason above.
    final Future<PickedFile?>? picking =
        widget.browser ? widget.pick() : null;
    final String name;
    final Uint8List bytes;
    try {
      if (picking != null) {
        final picked = await picking;
        // Closing the picker is a change of mind, not a failure.
        if (picked == null || !mounted) return;
        name = picked.name;
        bytes = picked.bytes;
      } else {
        name = path;
        bytes = await widget.readFile(path);
      }
    } catch (error) {
      if (mounted) {
        setState(() {
          _joinFailed = true;
          _joinNote = 'Could not read the file: $error';
        });
      }
      return;
    }
    if (!context.mounted) return;

    final agreed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Join with this file?'),
        content: Text(
          'Everything on this device is replaced by the records in $name — '
          'categories, budgets and receipts alike. Do this only on a device '
          'with nothing of its own yet.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Join'),
          ),
        ],
      ),
    );
    if (!(agreed ?? false) || !mounted) return;

    setState(() {
      _joining = true;
      _joinNote = null;
      _joinFailed = false;
      _joined = null;
    });
    try {
      final applied = await widget.join(bytes: bytes, phrase: phrase);
      if (!mounted) return;
      setState(() {
        _joined = applied;
        _joinFailed = false;
        _joinNote = 'Joined from $name. This device now carries the other '
            'device’s records, and the two keep up with each other by file '
            'from here: export to send what changes here, import what the '
            'other device sends back.';
      });
      await _load();
      await widget.onStandingChanged?.call();
    } catch (error) {
      // The core's refusals are sentences meant for this: the device has work
      // of its own, is already paired, or the file is an ordinary exchange
      // rather than a copy of everything. Nothing was replaced in any of them.
      if (mounted) {
        setState(() {
          _joinFailed = true;
          _joinNote = 'Nothing was changed. ${_sentence(_reason(error))}';
        });
      }
    } finally {
      if (mounted) setState(() => _joining = false);
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
        // First for a device that is not paired yet, because for one with
        // nothing of its own it is the first thing to do; and kept on screen
        // after a join, only to say how it went.
        if (status != null && (!status.isPaired || _joinNote != null))
          ..._joinSection(context, theme, status),
        const SizedBox(height: 24),
        Text('Export', style: theme.textTheme.titleSmall),
        const SizedBox(height: 4),
        Text(
          [
            widget.browser
                ? 'Puts everything the other device has not been sent into '
                    'one sealed file, which your browser downloads.'
                : 'Writes everything the other device has not been sent into '
                    'one sealed file.',
            'A device with nothing of its own yet — a new phone, or this app '
                'in a browser — needs a copy of everything instead: make it '
                'with For a new device, and join from it there.',
          ].join(' '),
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
        phraseField(
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
        if (!widget.browser) ...[
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
        ],
        const SizedBox(height: 12),
        // A Wrap rather than a Row: two buttons side by side are wider than a
        // phone held upright once the text is enlarged, and a Row would clip.
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: [
            FilledButton(
              onPressed: _exporting || _exportingWhole ? null : () => _doExport(),
              child: Text(
                widget.browser
                    ? (_exporting ? 'Preparing…' : 'Download')
                    : (_exporting ? 'Writing…' : 'Export'),
              ),
            ),
            OutlinedButton(
              key: const Key('export-whole'),
              onPressed: _exporting || _exportingWhole
                  ? null
                  : () => _doExportWhole(),
              child: Text(
                _exportingWhole
                    ? (widget.browser ? 'Preparing…' : 'Writing…')
                    : 'For a new device',
              ),
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
        if (_unconfirmed != null) ...[
          const SizedBox(height: 8),
          OutlinedButton(
            onPressed: _marking ? null : () => _confirmSent(),
            child: Text(_marking ? 'Recording…' : 'Mark as sent'),
          ),
        ],
        const SizedBox(height: 24),
        Text('Import', style: theme.textTheme.titleSmall),
        const SizedBox(height: 4),
        Text(
          [
            widget.browser
                ? 'Applies a file the other device wrote. Enter the phrase '
                    'first, then choose the file.'
                : 'Applies a file the other device wrote.',
            // An unpaired device that could join is the one that must not
            // import: both devices seeded the same categories under different
            // ids, so importing puts two of each side by side.
            if (status != null && !status.isPaired && _refusal == null)
              'This device is not paired yet, so if it has nothing of its own, '
                  'join with a file above instead — importing would put the '
                  'other device’s categories beside this one’s.',
          ].join(' '),
          style: theme.textTheme.bodySmall
              ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
        ),
        const SizedBox(height: 12),
        phraseField(
          fieldKey: const Key('import-phrase'),
          controller: _importPhrase,
          label: 'Pairing phrase',
          shown: _showImportPhrase,
          onToggle: () =>
              setState(() => _showImportPhrase = !_showImportPhrase),
          helper: 'The phrase the file was sealed with.',
        ),
        if (!widget.browser) ...[
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
        ],
        const SizedBox(height: 12),
        Row(
          children: [
            FilledButton.tonal(
              onPressed: _importing ? null : () => _doImport(),
              child: Text(
                _importing
                    ? 'Reading…'
                    : (widget.browser ? 'Choose file…' : 'Import'),
              ),
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
        if (_imported != null) ...importOutcome(theme, _imported!),
      ],
    );
  }

  /// Taking another device's records from a file, for a device with none of
  /// its own.
  ///
  /// Shown with its button off, and the core's reason beside it, when this
  /// device cannot join: a greyed button with no explanation reads as broken,
  /// and the reasons are ones a person can act on — export or merge the work
  /// that is here, or import rather than join a device it already syncs with.
  List<Widget> _joinSection(
    BuildContext context,
    ThemeData theme,
    SyncStatus status,
  ) {
    final refusal = _refusal;
    final offered = !status.isPaired;
    return [
      const SizedBox(height: 24),
      Text('Join with a file', style: theme.textTheme.titleSmall),
      // How to do it, only while it can still be done: once joined, the
      // outcome below is all there is left to say.
      if (offered) ...[
        const SizedBox(height: 4),
        Text(
          [
            'For a device with nothing of its own yet. It takes a copy of '
                'everything on the other device — categories, budgets and '
                'receipts — in place of what is here, and from then on the two '
                'keep up with each other by file. On the other device, make '
                'the file with For a new device.',
            if (widget.browser) 'Enter the phrase first, then choose the file.',
          ].join(' '),
          style: theme.textTheme.bodySmall
              ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
        ),
      ],
      if (offered && refusal != null) ...[
        const SizedBox(height: 12),
        Card(
          key: const Key('join-refusal'),
          margin: EdgeInsets.zero,
          color: theme.colorScheme.tertiaryContainer,
          child: Padding(
            padding: const EdgeInsets.all(12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  'This device can’t join',
                  style: theme.textTheme.titleSmall
                      ?.copyWith(color: theme.colorScheme.onTertiaryContainer),
                ),
                const SizedBox(height: 4),
                Text(
                  _sentence(refusal),
                  style: theme.textTheme.bodySmall
                      ?.copyWith(color: theme.colorScheme.onTertiaryContainer),
                ),
              ],
            ),
          ),
        ),
      ],
      // No boxes to fill in for a join that cannot happen; the button stays,
      // off, so the card above is plainly the reason it is off.
      if (offered && refusal == null) ...[
        const SizedBox(height: 12),
        phraseField(
          fieldKey: const Key('join-phrase'),
          controller: _joinPhrase,
          label: 'Pairing phrase',
          shown: _showJoinPhrase,
          onToggle: () => setState(() => _showJoinPhrase = !_showJoinPhrase),
          helper: 'The phrase the other device sealed the file with.',
        ),
        if (!widget.browser) ...[
          const SizedBox(height: 12),
          TextField(
            key: const Key('join-path'),
            controller: _joinPath,
            decoration: const InputDecoration(
              labelText: 'Read from',
              helperText: 'The full path of the file the other device wrote '
                  'for a new device.',
              helperMaxLines: 2,
              border: OutlineInputBorder(),
            ),
          ),
        ],
      ],
      if (offered) ...[
        const SizedBox(height: 12),
        FilledButton.tonal(
          key: const Key('join-button'),
          onPressed:
              _joining || refusal != null ? null : () => _doJoin(context),
          child: Text(
            _joining
                ? 'Joining…'
                : (widget.browser ? 'Choose file and join…' : 'Join'),
          ),
        ),
      ],
      if (_joinNote != null) ...[
        const SizedBox(height: 8),
        Semantics(
          liveRegion: true,
          child: Text(
            _joinNote!,
            style: theme.textTheme.bodyMedium?.copyWith(
              color: _joinFailed
                  ? theme.colorScheme.error
                  : theme.colorScheme.onSurfaceVariant,
            ),
          ),
        ),
      ],
      if (_joined != null) ...importOutcome(theme, _joined!),
    ];
  }

  /// A refusal as the core worded it, without the category label it files
  /// every refusal under. "invalid input:" is true of a file that holds only
  /// recent changes, and tells the person reading it nothing they can use.
  String _reason(Object error) {
    const label = 'invalid input: ';
    final text = '$error';
    return text.startsWith(label) ? text.substring(label.length) : text;
  }

  /// The core's reasons are clauses, written to follow "cannot join:"; on a
  /// card of their own they read as a sentence.
  String _sentence(String reason) {
    if (reason.isEmpty) return reason;
    final capital = reason[0].toUpperCase() + reason.substring(1);
    return capital.endsWith('.') ? capital : '$capital.';
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
          'No other device yet. If this one has nothing of its own, join '
          'another device’s copy with a file; otherwise export a file and '
          'import it there to start a pairing.',
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

}

/// What an import did, in full.
///
/// The three counts are the headline. Deferred changes and unknown kinds are
/// printed in full underneath rather than as a number, because both mean *data
/// did not arrive* and a count reads exactly like a clean import.
///
/// Top-level because two things import bundles now — a file somebody chose and
/// an exchange nobody watched — and the second one has more need of this than
/// the first: there was no human present to notice.
List<Widget> importOutcome(ThemeData theme, Applied applied) {
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
/// Obscured because what it protects is a whole ledger and somebody is often
/// standing next to the screen while a pairing is set up. Revealable because it
/// is typed from memory on two devices, and an unreadable field with no way to
/// check it turns one typo into "the phrase does not match" on the other
/// machine an hour later.
Widget phraseField({
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

/// A name for a copy of everything, made for a device that will join.
///
/// Named apart from an ordinary exchange file because the two look identical
/// in a downloads folder and only one of them can be joined from; the new
/// device's picker shows the name and nothing else.
String newDeviceFileName([DateTime? now]) =>
    defaultFileName(now).replaceFirst('pacc-', 'pacc-new-device-');

/// Keeping up with the other device over Wi-Fi, with nobody moving a file.
///
/// # Why there is a switch at all
///
/// Turning this on makes the device announce itself to every machine on the
/// network, several times a minute, and open a port that anything on that
/// network can connect to. Both are reasonable on a household's Wi-Fi and
/// neither is reasonable in a hotel, an office or a café — so it is a decision
/// somebody makes, not a default, and it is off until they make it.
///
/// # Why the phrase is stored here and nowhere else in this screen
///
/// The file half of sync asks for the phrase every time, because a human is
/// standing there each time a file is written. Nobody is standing there when a
/// receipt entered on a phone reaches a laptop, so the phrase has to be kept.
/// It is kept in the core's `app_setting` table — the same file as the ledger
/// it protects, which gives anybody who can read it nothing they could not
/// already read, and which never travels, because `app_setting` is not a
/// synced entity.
///
/// It is still never *shown*. The field below sets a new phrase and starts
/// empty even when one is saved: reading a stored secret back onto a screen is
/// how it ends up in a screenshot.
class LanSyncSection extends StatefulWidget {
  const LanSyncSection({super.key, required this.lan});

  /// The service itself, injected rather than reached for. It outlives this
  /// widget — it has to keep exchanging while the user is on another screen —
  /// so the section renders its state and never owns it.
  final LanSync lan;

  @override
  State<LanSyncSection> createState() => _LanSyncSectionState();
}

class _LanSyncSectionState extends State<LanSyncSection> {
  final TextEditingController _phrase = TextEditingController();
  bool _showPhrase = false;

  /// One control at a time while a setting is being written. Flicking the
  /// switch twice before the first write lands would leave the stored answer
  /// and the switch disagreeing about which won.
  bool _busy = false;

  @override
  void initState() {
    super.initState();
    // The Save button turns on the moment there is something to save, which
    // needs a rebuild per keystroke — cheap, and the alternative is a button
    // that looks available and does nothing.
    _phrase.addListener(_rebuild);
  }

  @override
  void dispose() {
    _phrase.removeListener(_rebuild);
    _phrase.dispose();
    super.dispose();
  }

  void _rebuild() => setState(() {});

  Future<void> _run(Future<void> Function() work) async {
    setState(() => _busy = true);
    try {
      await work();
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return ValueListenableBuilder<LanSyncState>(
      valueListenable: widget.lan.state,
      builder: (context, state, _) {
        final quiet = theme.textTheme.bodySmall
            ?.copyWith(color: theme.colorScheme.onSurfaceVariant);

        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Over Wi-Fi', style: theme.textTheme.titleSmall),
            const SizedBox(height: 4),
            Text(
              'The same exchange as the file above, done for you: while both '
              'devices are on this network they find each other and pass '
              'bundles back and forth, so an expense entered on one shows up '
              'on the other in a second or two. What crosses the network is '
              'the same sealed bundle, protected by the same phrase.',
              style: quiet,
            ),
            const SizedBox(height: 8),
            SwitchListTile(
              key: const Key('lan-switch'),
              contentPadding: EdgeInsets.zero,
              value: state.enabled,
              title: const Text('Sync over the local network'),
              subtitle: Text(_switchNote(state)),
              onChanged: _busy
                  ? null
                  : (on) => _run(() async {
                        if (!on) return widget.lan.turnOff();
                        await widget.lan.turnOn(_phrase.text);
                        // Saved now, so the field goes back to being a way to
                        // *change* the phrase rather than a copy of it left
                        // sitting on screen.
                        _phrase.clear();
                      }),
            ),
            const SizedBox(height: 8),
            phraseField(
              fieldKey: const Key('lan-phrase'),
              controller: _phrase,
              label: state.phraseIsSet ? 'New pairing phrase' : 'Pairing phrase',
              shown: _showPhrase,
              onToggle: () => setState(() => _showPhrase = !_showPhrase),
              helper: state.phraseIsSet
                  ? 'A phrase is already saved and is not shown. Type here only '
                      'to replace it — and then set the same one on the other '
                      'device, or they will stop understanding each other.'
                  : 'The same phrase on both devices. It is what every bundle '
                      'is encrypted with, and it is the only thing protecting '
                      'them on the way over.',
            ),
            const SizedBox(height: 12),
            FilledButton.tonal(
              key: const Key('lan-save-phrase'),
              onPressed: _busy || _phrase.text.isEmpty
                  ? null
                  : () => _run(() async {
                        await widget.lan.changePhrase(_phrase.text);
                        _phrase.clear();
                      }),
              child: Text(state.phraseIsSet ? 'Replace phrase' : 'Save phrase'),
            ),
            if (state.problem != null) ...[
              const SizedBox(height: 12),
              Semantics(
                liveRegion: true,
                child: Text(
                  state.problem!,
                  style: theme.textTheme.bodyMedium
                      ?.copyWith(color: theme.colorScheme.error),
                ),
              ),
            ],
            if (state.enabled) ...[
              const SizedBox(height: 20),
              Text('Devices on this network', style: theme.textTheme.labelMedium),
              const SizedBox(height: 4),
              if (state.peers.isEmpty)
                Text(
                  'Nothing heard yet. The other device needs to be on the same '
                  'Wi-Fi with this switched on; some networks — guest Wi-Fi, '
                  'most hotels — keep devices from seeing each other at all.',
                  style: quiet,
                )
              else
                for (final peer in state.peers)
                  ListTile(
                    key: Key('lan-peer-${peer.device}'),
                    contentPadding: EdgeInsets.zero,
                    leading: const Icon(Icons.devices),
                    title: Text(peer.device),
                    subtitle: Text('${peer.host}:${peer.port}', style: quiet),
                    trailing: state.paired || state.joinRefusal != null
                        ? null
                        : TextButton(
                            onPressed:
                                _busy ? null : () => _askToJoin(context, peer),
                            child: const Text('Join'),
                          ),
                  ),
              if (!state.paired) ..._pairingNote(theme, state),
            ],
            if (state.lastResult != null) ...[
              const SizedBox(height: 16),
              Semantics(
                liveRegion: true,
                child: Text(
                  '${state.lastResult!}${_when(state.lastAt)}',
                  style: theme.textTheme.bodyMedium?.copyWith(
                    color: state.lastFailed
                        ? theme.colorScheme.error
                        : theme.colorScheme.onSurfaceVariant,
                  ),
                ),
              ),
            ],
            // Only when something failed to land. A clean exchange has nothing
            // to report that the sentence above has not already said, and an
            // automatic sync that printed five counts every minute would train
            // everybody to ignore the one that mattered.
            if (state.lastApplied?.somethingIsMissing ?? false)
              ...importOutcome(theme, state.lastApplied!),
          ],
        );
      },
    );
  }

  /// What the switch is currently doing, which is not always what it says.
  String _switchNote(LanSyncState state) {
    if (!state.enabled) {
      return 'Off. This device announces nothing and answers nothing.';
    }
    if (!state.listening) {
      return 'On, but not running — see below.';
    }
    return state.paired
        ? 'Running. Changes go both ways as they happen.'
        : 'Running, but not paired with anything yet.';
  }

  /// Why nothing is being exchanged, and what to do about it.
  ///
  /// An unpaired device deliberately exchanges nothing on its own. Two devices
  /// that have each been opened have each invented their own ids for the same
  /// chart of accounts, and merging those leaves two of every category with
  /// the receipts split between them — which cannot be undone afterwards. So
  /// the first handshake is a person's decision, once, and everything after it
  /// is automatic.
  List<Widget> _pairingNote(ThemeData theme, LanSyncState state) {
    final refusal = state.joinRefusal;
    return [
      const SizedBox(height: 12),
      Card(
        margin: EdgeInsets.zero,
        color: theme.colorScheme.tertiaryContainer,
        child: Padding(
          padding: const EdgeInsets.all(12),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'Not paired yet',
                style: theme.textTheme.titleSmall
                    ?.copyWith(color: theme.colorScheme.onTertiaryContainer),
              ),
              const SizedBox(height: 4),
              Text(
                refusal == null
                    ? 'Nothing is exchanged automatically until these two '
                        'devices agree on one set of records. Press Join beside '
                        'the device your expenses are on: this one takes its '
                        'copy, and from then on they keep up with each other on '
                        'their own.'
                    : 'This device cannot take another’s copy: $refusal '
                        'Pair them with the export and import above instead, '
                        'and this section takes over once they have met.',
                style: theme.textTheme.bodySmall
                    ?.copyWith(color: theme.colorScheme.onTertiaryContainer),
              ),
            ],
          ),
        ),
      ),
    ];
  }

  /// Joining replaces everything here, so it asks first.
  Future<void> _askToJoin(BuildContext context, LanPeer peer) async {
    final agreed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Join ${peer.device}?'),
        content: const Text(
          'This device takes that one’s records and everything currently '
          'here is replaced — categories, budgets and receipts alike. Do this '
          'on the device that has nothing on it yet.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Join'),
          ),
        ],
      ),
    );
    if (agreed ?? false) await _run(() => widget.lan.joinPeer(peer));
  }

  /// The clock time of the last exchange.
  ///
  /// A time rather than "3 minutes ago": nothing here ticks, so a relative
  /// phrase would sit on screen going quietly stale.
  String _when(DateTime? at) {
    if (at == null) return '';
    String two(int n) => n.toString().padLeft(2, '0');
    return ' (${two(at.hour)}:${two(at.minute)}:${two(at.second)})';
  }
}

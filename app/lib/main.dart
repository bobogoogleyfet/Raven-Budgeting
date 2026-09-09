import 'dart:io';

import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';

import 'src/destinations.dart';
import 'dart:async';

import 'src/api.dart' show homeLocation, refreshRates;
import 'src/rust/api/db.dart';
import 'src/rust/frb_generated.dart';
import 'src/screens/budget_screen.dart';
import 'src/screens/entry_screen.dart';
import 'src/screens/expenses_screen.dart';
import 'src/screens/home_area_form.dart';
import 'src/screens/inbox_screen.dart';
import 'src/screens/settings_screen.dart';
import 'src/screens/home_screen.dart';
import 'src/shell.dart';
import 'src/sync/lan_sync.dart';
import 'src/sync/redraw.dart';
import 'src/theme.dart';

Future<void> main() async {
  await RustLib.init();

  // Desktop resolves the path in Rust, from the same environment variables the
  // CLI reads, so the app and `pacc` share one database.
  //
  // Mobile cannot: there is no shared location to share *with*, and the
  // per-app directory is the platform's answer — obtainable only through the
  // Java `Context`, which is what `path_provider` wraps. Support rather than
  // documents: this is a database the app manages, not a file the user is
  // meant to browse to, and on iOS the documents directory is backed up and
  // user-visible in ways a SQLite file with a WAL beside it should not be.
  final String? appData = Platform.isAndroid || Platform.isIOS
      ? (await getApplicationSupportDirectory()).path
      : null;

  final String path = await openDatabase(appDataDir: appData);
  // First run asks where the user lives before anything else.
  //
  // Detected by the home location still carrying a 0% rate, which is what the
  // seed installs: no rate means every receipt would record $0.00 of tax, and
  // the app cannot guess the right one.
  //
  // A failure here is not worth blocking entry over — the entry form will
  // surface the real error when it calls the same core.
  bool configured = false;
  try {
    final home = await homeLocation();
    configured = home != null && home.taxRatePpm > 0;
  } catch (_) {
    configured = false;
  }

  // Rates change on the first day of a quarter, so this does something four
  // times a year and nothing on the other 361 days. Deliberately not awaited:
  // the app must open instantly on all of them, including the ones where the
  // network is a captive portal that will hold a connection open for thirty
  // seconds before failing. Nothing on screen is waiting for the result, and a
  // failure is not an error — being offline is a laptop's normal state.
  unawaited(() async {
    try {
      await refreshRates();
    } catch (_) {
      // Being offline is a laptop's normal state, not a fault.
    }
  }());

  // Started before the first frame so a device that was told to sync is
  // already announcing itself by the time anybody looks at a screen, and so
  // whatever the other device did while this one was closed is on its way in.
  // Not awaited, for the same reason the rate refresh is not: a Wi-Fi that
  // will not carry a broadcast must cost nothing at launch, and the settings
  // screen reports whatever went wrong.
  final lan = LanSync();
  unawaited(lan.load());

  runApp(AccountingApp(databasePath: path, configured: configured, lan: lan));
}

class AccountingApp extends StatefulWidget {
  const AccountingApp({
    super.key,
    required this.databasePath,
    required this.configured,
    required this.lan,
  });

  final String databasePath;

  /// Exchanging changes with the other device over Wi-Fi. Owned here rather
  /// than by the settings screen: it has to keep running while the user is
  /// entering an expense, which is precisely when it has something to send.
  final LanSync lan;

  /// Whether the app has been told where home is. Until it has, there is
  /// nothing worth showing: every receipt entered would record no tax.
  final bool configured;

  @override
  State<AccountingApp> createState() => _AccountingAppState();
}

class _AccountingAppState extends State<AccountingApp> {
  late bool _configured = widget.configured;
  final GlobalKey<AppShellState> _shell = GlobalKey<AppShellState>();

  /// Bumped when something is written, to force the visible screen to reload.
  ///
  /// Every screen reads its data once, in `initState`, which is right for a
  /// screen nothing else writes to — and wrong the moment a receipt is filed
  /// behind it. Changing this changes the key the screen is built under, so
  /// its State is rebuilt and it asks the core again. A screen-by-screen
  /// refresh hook would be four hooks that each have to remember to fire.
  int _dataVersion = 0;

  /// Bumped by an exchange that brought something in, so a screen showing a
  /// list is rebuilt when a receipt lands from the other device.
  late final SyncRedraws _redraws = SyncRedraws(widget.lan.state);

  @override
  void initState() {
    super.initState();
    _redraws.addListener(_onSynced);
  }

  @override
  void dispose() {
    _redraws
      ..removeListener(_onSynced)
      ..dispose();
    super.dispose();
  }

  void _onSynced() {
    if (mounted) setState(() => _dataVersion++);
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Raven Budgeting',
      // The demo has to run on a debug build — `run-as`, which is the only
      // legal way to put the demo database where the Android app looks for
      // it, works on nothing else — so the banner cannot be dropped by
      // building for release. `scripts/demo.sh` passes this instead, and an
      // ordinary debug run still gets the banner.
      debugShowCheckedModeBanner: !const bool.fromEnvironment('PACC_DEMO'),
      theme: lightTheme(),
      darkTheme: darkTheme(),
      home: _configured
          ? AppShell(
              key: _shell,
              screenBuilder: _screenFor,
              onNewExpense: _newExpense,
            )
          : Scaffold(
              appBar: AppBar(title: const Text('Welcome')),
              body: HomeAreaForm(
                firstRun: true,
                submitLabel: 'Save and start',
                onSaved: () => setState(() => _configured = true),
              ),
            ),
    );
  }

  Widget _screenFor(BuildContext context, Destination destination) {
    return KeyedSubtree(
      key: ValueKey((destination, _dataVersion)),
      child: _destination(context, destination),
    );
  }

  Widget _destination(BuildContext context, Destination destination) {
    return switch (destination) {
      Destination.home =>
        HomeScreen(onSetBudget: _goToBudget, onOpenBudgets: _goToBudget),
      Destination.expenses => const ExpensesScreen(),
      Destination.budget => const BudgetScreen(),
      Destination.inbox => const InboxScreen(),
      Destination.settings =>
        SettingsScreen(databasePath: widget.databasePath, lan: widget.lan),
    };
  }

  /// The overview offers to set a budget; the Budget screen is where that
  /// lives, so this hands over rather than growing a second copy of it.
  void _goToBudget() => _shell.currentState?.show(Destination.budget);

  /// Entering an expense is a focused task, so it gets a screen of its own
  /// rather than a pane inside the shell — Material 3's full-screen dialog,
  /// which is also what makes it work identically on a phone.
  Future<void> _newExpense(BuildContext context) async {
    // Set by the form each time it writes one, which is not the same as the
    // form being closed: "Save & New" files several without ever leaving, and
    // closing without saving must not cost a reload.
    bool saved = false;
    final bool? goHome = await Navigator.of(context).push<bool>(
      MaterialPageRoute<bool>(
        fullscreenDialog: true,
        builder: (context) => Scaffold(
          appBar: AppBar(title: const Text('New expense')),
          body: EntryScreen(onSaved: () => saved = true),
        ),
      ),
    );
    // Before the destination changes, so whichever screen lands is already
    // showing the receipt that was just filed rather than the list from
    // before it.
    if (saved) {
      setState(() => _dataVersion++);
      // The same signal the screens reload on, sent to the other device. It
      // waits a second before doing anything, so filing three receipts with
      // "Save & New" is one exchange rather than three.
      widget.lan.nudge();
    }
    // The form pops `true` when the user chose "Save & Home". Dismissing it any
    // other way — the close button, Back, the system gesture — returns null and
    // leaves them wherever they were, which is the only answer that is right
    // for all of those.
    if (goHome ?? false) {
      _shell.currentState?.show(Destination.home);
    }
  }
}


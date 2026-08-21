/// The bridge boundary.
///
/// Generated bindings in, app types out. This is the only file that imports
/// `src/rust/`, and the only place [fromCore] is called — so every monetary
/// value in the app is traceable to something the core computed.
///
/// It also absorbs one ergonomic wrinkle: `Vec<i64>` arguments arrive in Dart
/// as `Int64List`. Converting once here beats every caller remembering to.
library;

import 'dart:typed_data' show Uint8List;

import 'package:flutter_rust_bridge/flutter_rust_bridge.dart' show Int64List;

import 'money.dart';
import 'rust/api/budget.dart' as rust_budget;
import 'rust/api/entry.dart' as rust_entry;
import 'rust/api/rates.dart' as rust_rates;
import 'rust/api/categories.dart' as rust_categories;
import 'rust/api/sync.dart' as rust_sync;
import 'rust/api/transactions.dart' as rust_txn;

export 'rust/api/categories.dart' show CategoryDto, CategoryGroupDto;
export 'rust/api/entry.dart'
    show AreaDto, ContextDto, LocationDto, MerchantDto, RateReleaseDto;
export 'rust/api/rates.dart' show RefreshDto;

/// One row of the expense list.
class TxnSummary {
  const TxnSummary({
    required this.id,
    required this.txnDate,
    required this.merchantName,
    required this.total,
    required this.taxIsEstimated,
    required this.needsReview,
    required this.categories,
  });

  final int id;

  /// `YYYY-MM-DD`, as the core formatted it. Rendered as-is: parsing it into a
  /// `DateTime` and formatting it again is how a UTC shift gets reintroduced
  /// on a screen that never needed a timezone.
  final String txnDate;

  final String? merchantName;
  final Cents total;
  final bool taxIsEstimated;
  final bool needsReview;
  final List<String> categories;
}

/// One category's budget for a month.
class BudgetRow {
  const BudgetRow({
    required this.categoryId,
    required this.categoryName,
    required this.groupName,
    required this.budgeted,
    required this.perMonth,
    required this.funded,
    required this.spentCycle,
    required this.remaining,
    required this.overspentBy,
    required this.spentThisMonth,
    required this.groupIsObligation,
    required this.contributedThisMonth,
    required this.isSinkingFund,
    required this.funding,
    required this.fundingBy,
    required this.monthsRemaining,
  });

  final int categoryId;
  final String categoryName;
  final String groupName;
  final Cents? budgeted;
  final Cents? perMonth;
  final Cents? funded;
  final Cents? spentCycle;

  /// What is left of this cycle's budget. Negative once spending passes it.
  final Cents? remaining;

  /// Zero while within budget; any other value is the over-budget signal.
  final Cents? overspentBy;

  final Cents spentThisMonth;

  /// Money budgeted in this category's group is owed, not merely available.
  ///
  /// Rent unpaid is being behind; groceries unspent is having money left. The
  /// core decides this from the group rather than each screen guessing from a
  /// name.
  final bool groupIsObligation;

  /// What was actually set aside this month. `null` means nothing has been
  /// recorded, which a sinking fund counts as nothing saved — distinct from a
  /// recorded zero, which is somebody saying so out loud.
  final Cents? contributedThisMonth;

  /// Saved into rather than paid out of: a yearly premium, a car-repair fund.
  /// Only these take contributions.
  final bool isSinkingFund;

  /// `"ahead" | "on track" | "behind" | "none"`.
  final String funding;
  final Cents fundingBy;
  final int? monthsRemaining;

  /// Whether this category has a budget line at all in the month asked about.
  bool get isBudgeted => perMonth != null;

  /// Over budget — which is **not** the same question as behind on funding.
  /// A $900 rent line nobody has paid yet is behind on funding while having
  /// spent nothing at all, and labelling that "over budget" would flag every
  /// bill that is merely still due.
  bool get isOver => (overspentBy?.value ?? 0) != 0;

  /// A cycle longer than one month, so "left" is left of a year's bill rather
  /// than of this month's.
  bool get isMultiMonth => (monthsRemaining ?? 1) > 1;
}

/// What is budgeted this month, and what has been spent against it.
///
/// Summed by the core. A frontend that added the rows up itself would be doing
/// money arithmetic, and would drift from every other frontend that did the
/// same.
class MonthTotals {
  const MonthTotals({required this.budgeted, required this.spent});

  final Cents budgeted;
  final Cents spent;
}

/// One value a filterable column actually holds.
class FilterValue {
  const FilterValue({required this.id, required this.name});

  final int id;
  final String name;
}

class FilterValues {
  const FilterValues({
    required this.merchants,
    required this.categories,
    required this.anyWithoutMerchant,
  });

  final List<FilterValue> merchants;
  final List<FilterValue> categories;
  final bool anyWithoutMerchant;
}

/// How the expense list is ordered. The spellings the core accepts.
enum TxnSort {
  dateDesc('dateDesc'),
  dateAsc('dateAsc'),
  totalDesc('totalDesc'),
  totalAsc('totalAsc');

  const TxnSort(this.wire);

  final String wire;
}

/// What to ask the expense list for.
///
/// `categoryIds` and `merchantIds` are `null` rather than "a list of all of
/// them", because the two differ the moment a category is added: an explicit
/// list built yesterday would hide anything filed under a category created
/// since.
class TxnQuery {
  const TxnQuery({
    required this.limit,
    this.offset = 0,
    this.categoryIds,
    this.merchantIds,
    this.includeNoMerchant = false,
    this.sort = TxnSort.dateDesc,
  });

  /// How many rows one page holds.
  final int limit;

  /// How many matching rows to skip: page 3 of 50 asks for `offset: 100`.
  ///
  /// Paging is done in SQL rather than by loading everything and slicing it
  /// here, so the app never holds a table it is not showing.
  final int offset;

  final List<int>? categoryIds;
  final List<int>? merchantIds;
  final bool includeNoMerchant;
  final TxnSort sort;

  /// The same filter, pointed at a different page.
  TxnQuery atOffset(int offset) => TxnQuery(
        limit: limit,
        offset: offset,
        categoryIds: categoryIds,
        merchantIds: merchantIds,
        includeNoMerchant: includeNoMerchant,
        sort: sort,
      );
}

/// The expense list's bottom line: the money columns of the whole filtered
/// set, added up, and how many expenses that was.
///
/// **The whole filtered set, not the page on screen.** The useful figure is
/// "everything matching this filter", and it must not move as the reader
/// presses Next — a total that changed per page would read as an arithmetic
/// bug, because that is what it would look like.
class TxnTotals {
  const TxnTotals({
    required this.subtotal,
    required this.tax,
    required this.total,
    required this.expenses,
  });

  final Cents subtotal;
  final Cents tax;
  final Cents total;

  /// Expenses in the sum. Also the number the pager counts through.
  final int expenses;
}

/// `flutter_rust_bridge`'s own `Int64List`, not `dart:typed_data`'s.
///
/// They are different classes with the same name — the bridge's is a strict
/// wrapper that yields `BigInt` on read — and passing the wrong one is a type
/// error naming the same type twice, which is a genuinely baffling message
/// until you know. Only ids go through here; money never does.
Int64List? _ids(List<int>? ids) =>
    ids == null ? null : Int64List.fromList(ids);

/// Every category that is not archived.
Future<List<rust_categories.CategoryDto>> listCategories() =>
    rust_categories.listCategories();

/// The expense list.
Future<List<TxnSummary>> listTransactions(TxnQuery query) async {
  final rows = await rust_txn.listTransactions(
    limit: query.limit,
    offset: query.offset,
    categoryIds: _ids(query.categoryIds),
    merchantIds: _ids(query.merchantIds),
    includeNoMerchant: query.includeNoMerchant,
    sort: query.sort.wire,
  );
  return rows
      .map(
        (r) => TxnSummary(
          id: r.id,
          txnDate: r.txnDate,
          merchantName: r.merchantName,
          total: fromCore(r.total),
          taxIsEstimated: r.taxIsEstimated,
          needsReview: r.needsReview,
          categories: r.categories,
        ),
      )
      .toList();
}

/// Add up every expense a filter matches, however the list is paged.
///
/// `limit` and `offset` are deliberately not sent: a page total would change
/// under a reader who only pressed Next. Summed in the core, from the same
/// filter the list is built from, so the row of totals can never describe a
/// different set from the rows above it. Not here — [Cents] makes arithmetic a
/// compile error on purpose, so a total is never a figure this layer invented.
Future<TxnTotals> transactionTotals(TxnQuery query) async {
  final t = await rust_txn.transactionTotals(
    categoryIds: _ids(query.categoryIds),
    merchantIds: _ids(query.merchantIds),
    includeNoMerchant: query.includeNoMerchant,
  );
  return TxnTotals(
    subtotal: fromCore(t.subtotal),
    tax: fromCore(t.tax),
    total: fromCore(t.total),
    expenses: t.expenses,
  );
}

/// The values the filter menus are built from.
Future<FilterValues> filterValues() async {
  final values = await rust_txn.filterValues();
  FilterValue named(rust_txn.FilterValueDto v) =>
      FilterValue(id: v.id, name: v.name);
  return FilterValues(
    merchants: values.merchants.map(named).toList(),
    categories: values.categories.map(named).toList(),
    anyWithoutMerchant: values.anyWithoutMerchant,
  );
}

/// Delete a receipt. Throws if it is already gone, which is worth surfacing
/// rather than swallowing: two windows open on one database is normal.
Future<void> deleteTransaction(int id) => rust_txn.deleteTransaction(id: id);

/// The month's totals, for the figure the overview leads with.
Future<MonthTotals> monthTotals(String month) async {
  final totals = await rust_budget.monthTotals(month: month);
  return MonthTotals(
    budgeted: fromCore(totals.budgeted),
    spent: fromCore(totals.spent),
  );
}

/// Every budgeted category, for a month given as `YYYY-MM`.
Future<List<BudgetRow>> budgetStatus(String month) async {
  final rows = await rust_budget.budgetStatus(month: month);
  Cents? cents(int? value) => value == null ? null : fromCore(value);
  return rows
      .map(
        (r) => BudgetRow(
          categoryId: r.categoryId,
          categoryName: r.categoryName,
          groupName: r.groupName,
          budgeted: cents(r.budgeted),
          perMonth: cents(r.perMonth),
          funded: cents(r.funded),
          spentCycle: cents(r.spentCycle),
          remaining: cents(r.remaining),
          overspentBy: cents(r.overspentBy),
          spentThisMonth: fromCore(r.spentThisMonth),
          groupIsObligation: r.groupIsObligation,
          contributedThisMonth: cents(r.contributedThisMonth),
          isSinkingFund: r.isSinkingFund,
          funding: r.funding,
          fundingBy: fromCore(r.fundingBy),
          monthsRemaining: r.monthsRemaining,
        ),
      )
      .toList();
}

/// The budget screen's bottom line: every column of the table, added up.
class BudgetTotals {
  const BudgetTotals({
    required this.perMonth,
    required this.funded,
    required this.spentCycle,
    required this.remaining,
    required this.overspentBy,
    required this.categories,
  });

  final Cents perMonth;
  final Cents funded;
  final Cents spentCycle;
  final Cents remaining;
  final Cents overspentBy;

  /// Budgeted categories in the total, so it can be checked against the table.
  final int categories;
}

/// Add up a month's budget table.
///
/// Summed in the core, from the same rows [budgetStatus] returns. Not here:
/// [Cents] makes arithmetic a compile error on purpose, so a total is never a
/// figure this layer invented.
Future<BudgetTotals> budgetTotals(String month) async {
  final t = await rust_budget.budgetTotals(month: month);
  return BudgetTotals(
    perMonth: fromCore(t.perMonth),
    funded: fromCore(t.funded),
    spentCycle: fromCore(t.spentCycle),
    remaining: fromCore(t.remaining),
    overspentBy: fromCore(t.overspentBy),
    categories: t.categories,
  );
}

/// Record what was actually set aside for a category in one month.
///
/// Not an expense: money moved into a sinking fund has not been spent, and
/// filing it as spending would count it against the budget now and again when
/// the bill it is for is finally paid.
Future<void> recordContribution({
  required int categoryId,
  required String month,
  required Cents amount,
}) =>
    rust_budget.recordContribution(
      categoryId: categoryId,
      month: month,
      amountCents: amount.value,
    );

/// Forget a recorded contribution, returning the month to "nothing recorded".
Future<void> forgetContribution({
  required int categoryId,
  required String month,
}) =>
    rust_budget.forgetContribution(categoryId: categoryId, month: month);

/// One category's line on the receipt being entered.
class DraftRow {
  const DraftRow({
    required this.categoryId,
    required this.subtotal,
    this.tax,
    this.note,
  });

  final int? categoryId;
  final Cents subtotal;

  /// The tax the user read off the paper. `null` — the normal case — leaves it
  /// to the rate, and is what keeps `taxIsEstimated` meaning something.
  final Cents? tax;

  final String? note;
}

/// A receipt as it is being typed.
class SubtotalDraft {
  const SubtotalDraft({
    required this.txnDate,
    required this.rows,
    this.merchant,
    this.location,
    this.notes,
  });

  /// `YYYY-MM-DD`, from the local clock. The core is not allowed to read it.
  final String txnDate;
  final List<DraftRow> rows;
  final String? merchant;

  /// Null uses the merchant's remembered location, else home.
  final String? location;
  final String? notes;

  rust_entry.SubtotalDraftDto _dto() => rust_entry.SubtotalDraftDto(
        id: null,
        txnDate: txnDate,
        merchant: merchant,
        location: location,
        rows: [
          for (final row in rows)
            rust_entry.SubtotalRowDto(
              categoryId: row.categoryId,
              subtotal: row.subtotal.value,
              tax: row.tax?.value,
              note: row.note,
            ),
        ],
        notes: notes,
        needsReview: false,
        rememberDefaults: true,
      );
}

/// One row of a preview: what would be stored against that category.
class PreviewRow {
  const PreviewRow({
    required this.categoryId,
    required this.categoryName,
    required this.subtotal,
    required this.tax,
    required this.amount,
  });

  final int? categoryId;
  final String? categoryName;
  final Cents subtotal;
  final Cents tax;
  final Cents amount;
}

/// What would be written, and why it might not be.
class Preview {
  const Preview({
    required this.subtotal,
    required this.tax,
    required this.total,
    required this.taxIsEstimated,
    required this.taxRatePpm,
    required this.appliedRatePpm,
    required this.rows,
    required this.warnings,
    required this.blockers,
  });

  final Cents subtotal;
  final Cents tax;
  final Cents total;
  final bool taxIsEstimated;

  /// The location's general rate. Not necessarily what this receipt was taxed
  /// at, and never the number to label a tax figure with.
  final int taxRatePpm;

  /// The rate every taxed row actually used, when they all used one. Null when
  /// the receipt straddles two — groceries at 3% and shampoo at 7.45% in the
  /// same run — because then no single figure is right for all of it.
  final int? appliedRatePpm;

  final List<PreviewRow> rows;

  /// Worth showing; saving is still allowed.
  final List<String> warnings;

  /// Saving is impossible until these are empty.
  final List<String> blockers;

  bool get canSave => blockers.isEmpty;
}

Preview _preview(rust_entry.PreviewDto dto) => Preview(
      subtotal: fromCore(dto.subtotal),
      tax: fromCore(dto.tax),
      total: fromCore(dto.total),
      taxIsEstimated: dto.taxIsEstimated,
      taxRatePpm: dto.taxRatePpm,
      appliedRatePpm: dto.appliedRatePpm,
      rows: [
        for (final row in dto.rows)
          PreviewRow(
            categoryId: row.categoryId,
            categoryName: row.categoryName,
            subtotal: fromCore(row.subtotal),
            tax: fromCore(row.tax),
            amount: fromCore(row.amount),
          ),
      ],
      warnings: dto.warnings,
      blockers: dto.blockers,
    );

/// Reconcile a receipt without writing anything.
///
/// Safe to call on every keystroke, and the only way the app learns what a
/// receipt's numbers are. Saving runs the same code first, so the figures on
/// screen are what will be stored.
Future<Preview> previewSubtotals(SubtotalDraft draft) async =>
    _preview(await rust_entry.previewSubtotals(draft: draft._dto()));

/// Write the receipt, returning its id.
Future<int> saveSubtotals(SubtotalDraft draft) =>
    rust_entry.saveSubtotals(draft: draft._dto());

/// What the app already knows about a store.
Future<rust_entry.ContextDto> entryContext(String? merchant) =>
    rust_entry.entryContext(merchant: merchant);

/// Make a location real so a draft can name it. The core refuses a location it
/// does not know, and falling back to "no location" would switch the rate to
/// 0% and put a plausible, wrong number on screen.
Future<rust_entry.LocationDto> ensureLocation(String name) =>
    rust_entry.ensureLocation(name: name);

/// The home location, or null when the app has not been told where that is.
Future<rust_entry.LocationDto?> homeLocation() => rust_entry.homeLocation();

Future<List<rust_entry.MerchantDto>> listMerchants() => rust_entry.listMerchants();

Future<List<rust_entry.LocationDto>> listLocations() => rust_entry.listLocations();

/// Every jurisdiction the state publishes.
Future<List<rust_entry.AreaDto>> listAreas() => rust_entry.listAreas();

/// The needs-categorizing queue, newest first.
///
/// Not "everything filed under Uncategorized": the queue is what `quick-add`
/// flagged for review, which is a different set — a hand-entered receipt left
/// uncategorized was never flagged, and a quick-add whose store had a
/// remembered category still needs looking at.
Future<List<TxnSummary>> needsCategorizing() async {
  final rows = await rust_entry.needsCategorizing();
  return [
    for (final r in rows)
      TxnSummary(
        id: r.id,
        txnDate: r.txnDate,
        merchantName: r.merchantName,
        total: fromCore(r.total),
        taxIsEstimated: r.taxIsEstimated,
        needsReview: r.needsReview,
        categories: r.categories,
      ),
  ];
}

/// File a quick-added receipt under one category.
///
/// Sent as the remainder rather than as the total this screen happens to be
/// showing: the core assigns the exact amount on disk, and the two differ the
/// moment another window edits the row.
Future<void> categorizeWhole(int id, int categoryId) => rust_entry.categorize(
      id: id,
      splits: [
        rust_entry.SplitDraftDto(categoryId: categoryId, cents: null, note: null),
      ],
    );

/// Name the place the user lives and set its sales-tax rates.
///
/// A hand-typed rate must send a null `areaCode` *and* null rates beside it:
/// the user moved the choice off the list, so there is no jurisdiction to
/// follow, and carrying Utah's 3% over to Boise would undertax every grocery
/// receipt entered there — as carrying Utah County's restaurant tax would
/// overtax every meal.
///
/// Every rate is `required` even though each is nullable, so that adding one
/// is a compile error at each call site rather than a silently omitted named
/// argument. A dropped `restaurantRatePpm` would fail nowhere; it would just
/// quietly go back to taxing meals 1% light.
Future<void> setHomeLocation({
  required String name,
  required String? areaCode,
  required int taxRatePpm,
  required int? foodRatePpm,
  required int? restaurantRatePpm,
}) =>
    rust_entry.setHomeLocation(
      name: name,
      areaCode: areaCode,
      taxRatePpm: taxRatePpm,
      foodRatePpm: foodRatePpm,
      restaurantRatePpm: restaurantRatePpm,
    );

/// Which rate publication is installed, or null if none is.
Future<rust_entry.RateReleaseDto?> rateRelease() => rust_entry.rateRelease();

/// Refresh the rate table if the quarter has turned.
///
/// Never throws for being offline: a laptop that cannot reach the network is in its
/// normal state, and last quarter's rates are estimates a receipt overrules
/// anyway. The staleness is what gets surfaced, by [rateRelease].
Future<rust_rates.RefreshDto> refreshRates() => rust_rates.refreshRates();

/// What setting a budget did.
class Superseded {
  const Superseded({required this.restated, required this.closedAfter});

  /// True when a line already starting that month was restated in place.
  final bool restated;

  /// The last month the previous line now applies to, when one was closed to
  /// make room for this.
  final String? closedAfter;
}

/// How often a budgeted amount comes due.
enum BudgetPeriod {
  monthly('monthly', 'Every month'),
  yearly('yearly', 'Once a year');

  const BudgetPeriod(this.wire, this.label);

  final String wire;
  final String label;
}

/// Set a category's budget from `month` onward.
///
/// The amount is for one **period**, not per month: a yearly budget of $1,200
/// contributes about $100 a month, and the core does that division.
Future<Superseded> setBudget({
  required int categoryId,
  required Cents amount,
  required BudgetPeriod period,
  required String month,
  bool rollover = false,
}) async {
  final result = await rust_budget.setBudget(
    categoryId: categoryId,
    amountCents: amount.value,
    period: period.wire,
    everyNMonths: null,
    month: month,
    rollover: rollover,
  );
  return Superseded(
    restated: result.restated,
    closedAfter: result.closedAfter,
  );
}

/// The groups a category can belong to, without the hidden System one — the
/// three categories the app itself depends on are no answer to "where does
/// this belong".
Future<List<rust_categories.CategoryGroupDto>> listCategoryGroups() async {
  final groups = await rust_categories.listCategoryGroups();
  return [
    for (final group in groups)
      if (!group.isHidden) group,
  ];
}

/// Add a category.
///
/// `taxable` says whether a till adds sales tax to it. There is no sensible
/// default: a car payment taxed at 7.45% is $537.25 where the statement says
/// $500.00, and nothing on screen would say why.
Future<rust_categories.CategoryDto> createCategory({
  required int groupId,
  required String name,
  required bool taxable,
}) =>
    rust_categories.createCategory(
      groupId: groupId,
      name: name,
      taxable: taxable,
    );

/// Add a group for categories to go in.
Future<rust_categories.CategoryGroupDto> createCategoryGroup(String name) =>
    rust_categories.createCategoryGroup(name: name);

/// Add up amounts typed as separate figures.
///
/// Synchronous: it reads no database, so a field can total as it is typed
/// rather than a frame later. The adding is the core's — a frontend that adds
/// money has to do it again for every platform, and the copies drift.
Cents totalOf(List<Cents> parts) => fromCore(
      rust_entry.totalOf(
        parts: Int64List.fromList([for (final part in parts) part.value]),
      ),
    );

/// Where this database has got to with the devices it syncs with.
///
/// [thisDevice] names the *database*, not the hardware and not the person: a
/// peer tracks how far along this one is by that name, so a database restored
/// onto a new laptop carries on as the same device rather than becoming a
/// third one.
class SyncStatus {
  const SyncStatus({
    required this.thisDevice,
    required this.peers,
    required this.openConflicts,
  });

  final String thisDevice;

  /// Every device this one has exchanged anything with. Empty until a bundle
  /// from somewhere else has been imported — which is what a first export is
  /// for.
  final List<String> peers;

  /// Disagreements nobody has looked at yet. Not errors: both devices changed
  /// the same thing, the later edit is in place, and the earlier one is kept.
  final int openConflicts;

  /// Whether an export has a peer to address itself to.
  bool get isPaired => peers.isNotEmpty;
}

/// What importing a bundle did.
///
/// The counts are the headline; [deferred] and [unknownKinds] are the part
/// that must never be shown as a count alone. Both mean *data did not arrive*,
/// and a bare number reads exactly like a clean import.
class Applied {
  const Applied({
    required this.applied,
    required this.ignored,
    required this.conflicts,
    required this.deferred,
    required this.unknownKinds,
  });

  /// Changes taken, because they were newer than anything here.
  final int applied;

  /// Changes ignored, because this device already had the same or newer.
  /// Nothing was lost: the other device was simply behind.
  final int ignored;

  /// Changes where both devices had edited the same thing. The later edit won
  /// and the earlier one was kept rather than dropped — losing a conflict is
  /// not a judgement that the value was wrong.
  final int conflicts;

  /// Changes that could not be applied because something they point at has not
  /// reached this device. One sentence each, from the core.
  final List<String> deferred;

  /// Kinds this release does not understand, written by a newer one.
  final List<String> unknownKinds;

  /// Whether anything in the file failed to land. Both lists mean the same
  /// thing to a user — it is still only on the other device.
  bool get somethingIsMissing => deferred.isNotEmpty || unknownKinds.isNotEmpty;
}

/// This device's name within the sync group, minting one if it has never had
/// one.
Future<String> thisDevice() => rust_sync.thisDevice();

/// This device, its peers, and anything waiting for a human.
Future<SyncStatus> syncStatus() async {
  final status = await rust_sync.syncStatus();
  return SyncStatus(
    thisDevice: status.thisDevice,
    peers: status.peers,
    openConflicts: status.openConflicts,
  );
}

/// Build the sealed file holding everything a peer has not been sent.
///
/// Returns bytes rather than writing them: on Android and iOS the destination
/// comes from a system picker and the app is handed a stream, so where the
/// file goes is the UI's business. Nothing is recorded as sent by this —
/// [markDelivered] is the separate step, taken once the bytes are actually on
/// disk, because a file that never reached the other device must not stop its
/// contents from being sent again.
///
/// A null [peer] means "the only device you sync with"; with several the core
/// refuses and names them rather than send one device's history to another.
Future<Uint8List> exportBundle({String? peer, required String phrase}) =>
    rust_sync.exportBundle(peer: peer, phrase: phrase);

/// Record that a bundle reached its peer, so the next one carries only what
/// changed after it.
///
/// Does nothing when this database has no peer yet: a watermark names a
/// device, and the first bundle of a pairing has nobody to name.
Future<void> markDelivered({
  String? peer,
  required String phrase,
  required Uint8List bytes,
}) =>
    rust_sync.markDelivered(peer: peer, phrase: phrase, bytes: bytes);

/// Apply a sealed file written by another device.
///
/// Throws when the phrase does not match or the file is not a sync file. The
/// message names neither the phrase nor anything inside the file.
Future<Applied> importBundle({
  required Uint8List bytes,
  required String phrase,
}) async {
  final result = await rust_sync.importBundle(bytes: bytes, phrase: phrase);
  return Applied(
    applied: result.applied,
    ignored: result.ignored,
    conflicts: result.conflicts,
    deferred: result.deferred_,
    unknownKinds: result.unknownKinds,
  );
}

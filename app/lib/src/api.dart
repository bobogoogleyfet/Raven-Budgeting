/// The bridge boundary.
///
/// Generated bindings in, app types out. This is the only file that imports
/// `src/rust/`, and the only place [fromCore] is called — so every monetary
/// value in the app is traceable to something the core computed.
///
/// It also absorbs one ergonomic wrinkle: `Vec<i64>` arguments arrive in Dart
/// as `Int64List`. Converting once here beats every caller remembering to.
library;

import 'package:flutter_rust_bridge/flutter_rust_bridge.dart' show Int64List;

import 'money.dart';
import 'rust/api/budget.dart' as rust_budget;
import 'rust/api/entry.dart' as rust_entry;
import 'rust/api/rates.dart' as rust_rates;
import 'rust/api/categories.dart' as rust_categories;
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

  /// `"ahead" | "on track" | "behind" | "none"`.
  final String funding;
  final Cents fundingBy;
  final int? monthsRemaining;

  /// Whether this category has a budget line at all in the month asked about.
  bool get isBudgeted => perMonth != null;

  /// Over budget — which is **not** the same question as behind on funding.
  /// An envelope spent exactly to plan is behind on funding and perfectly fine
  /// on spending, and labelling that "over budget" would flag every well-run
  /// category.
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
    this.categoryIds,
    this.merchantIds,
    this.includeNoMerchant = false,
    this.sort = TxnSort.dateDesc,
  });

  final int limit;
  final List<int>? categoryIds;
  final List<int>? merchantIds;
  final bool includeNoMerchant;
  final TxnSort sort;
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
          funding: r.funding,
          fundingBy: fromCore(r.fundingBy),
          monthsRemaining: r.monthsRemaining,
        ),
      )
      .toList();
}

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

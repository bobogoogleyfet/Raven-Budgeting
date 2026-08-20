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
import 'rust/api/categories.dart' as rust_categories;
import 'rust/api/transactions.dart' as rust_txn;

export 'rust/api/categories.dart' show CategoryDto;

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

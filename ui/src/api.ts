/**
 * The typed IPC surface.
 *
 * Every one of these mirrors a function in `accounting-core`'s service layer,
 * and the types mirror its structs field for field. Two consequences are
 * deliberate:
 *
 * 1. **Nothing here computes anything.** `preview` sends a draft and gets back
 *    the amounts, the resolved splits, the warnings and the blockers — all
 *    computed in Rust by the same code `save` will use. That is what makes it
 *    impossible for the UI to show one set of numbers and store another, and
 *    it is why there is no `subtotal = total - tax` anywhere in this app.
 * 2. **Money crosses as integer cents.** `Money` is `serde(transparent)` over
 *    `i64`, so it arrives as a plain JSON integer and is branded on arrival by
 *    {@link fromCore}. See `money.ts` for why that brand exists.
 *
 * When a second frontend is written for Android, this file is the part that is
 * rewritten; everything it calls stays exactly as it is.
 */

import { invoke } from "@tauri-apps/api/core";

import { type Cents, fromCore } from "./money.ts";

/** `'grouped' | 'itemized'` — how much detail a receipt records. */
export type DetailLevel = "grouped" | "itemized";

/** A split's share, as the user expressed it. Mirrors `domain::SplitAmount`. */
export type SplitAmount =
  | { kind: "fixed"; cents: number }
  /** Basis points: 25% is `2500`. Integers, so a third of a receipt is exact. */
  | { kind: "percent"; bp: number }
  | { kind: "remainder" };

export interface SplitDraft {
  categoryId: number | null;
  amount: SplitAmount;
  note: string | null;
}

export interface TransactionDraft {
  id: number | null;
  txnDate: string;
  merchant: string | null;
  location: string | null;
  subtotal: number | null;
  tax: number | null;
  total: number | null;
  splits: SplitDraft[];
  detailLevel: DetailLevel | null;
  notes: string | null;
  needsReview: boolean;
  rememberDefaults: boolean;
}

/** Raw shapes as they arrive over IPC, before branding. */
interface RawSplitRow {
  categoryId: number | null;
  categoryName: string | null;
  subtotal: number;
  tax: number;
  amount: number;
}

interface RawPreview {
  subtotal: number;
  tax: number;
  total: number;
  taxIsEstimated: boolean;
  taxRatePpm: number;
  unassigned: number;
  detailLevel: DetailLevel;
  rows: RawSplitRow[];
  warnings: string[];
  blockers: string[];
}

export interface SplitRow {
  categoryId: number | null;
  categoryName: string | null;
  subtotal: Cents;
  tax: Cents;
  amount: Cents;
}

export interface Preview {
  subtotal: Cents;
  tax: Cents;
  total: Cents;
  /** True only when the core computed the tax rather than reading it off a
   * receipt. Render it — an estimate that looks like a fact is worse than no
   * estimate. */
  taxIsEstimated: boolean;
  taxRatePpm: number;
  /** Money not yet assigned to a category. Non-zero blocks saving. */
  unassigned: Cents;
  detailLevel: DetailLevel;
  rows: SplitRow[];
  /** Worth showing; saving is still allowed. */
  warnings: string[];
  /** Saving is impossible until these are empty. */
  blockers: string[];
}

export interface Category {
  id: number;
  name: string;
  groupName: string;
  isBudgeted: boolean;
}

interface RawTxnSummary {
  id: number;
  txnDate: string;
  merchantName: string | null;
  total: number;
  taxIsEstimated: boolean;
  needsReview: boolean;
  categories: string[];
}

export interface TxnSummary {
  id: number;
  txnDate: string;
  merchantName: string | null;
  total: Cents;
  taxIsEstimated: boolean;
  needsReview: boolean;
  categories: string[];
}

function brandSplit(row: RawSplitRow): SplitRow {
  return {
    categoryId: row.categoryId,
    categoryName: row.categoryName,
    subtotal: fromCore(row.subtotal),
    tax: fromCore(row.tax),
    amount: fromCore(row.amount),
  };
}

function brandPreview(raw: RawPreview): Preview {
  return {
    ...raw,
    subtotal: fromCore(raw.subtotal),
    tax: fromCore(raw.tax),
    total: fromCore(raw.total),
    unassigned: fromCore(raw.unassigned),
    rows: raw.rows.map(brandSplit),
  };
}

function brandSummary(raw: RawTxnSummary): TxnSummary {
  return { ...raw, total: fromCore(raw.total) };
}

/** The chart of accounts, for the category picker. */
export async function listCategories(): Promise<Category[]> {
  return await invoke<Category[]>("list_categories");
}

/**
 * Reconcile and plan a draft **without writing anything**.
 *
 * Safe to call on every keystroke. This is the only way the UI learns what a
 * receipt's numbers are.
 */
export async function previewTransaction(draft: TransactionDraft): Promise<Preview> {
  return brandPreview(await invoke<RawPreview>("preview_transaction", { draft }));
}

/** Persist a draft. Rejects if the preview had any blocker. Returns the id. */
export async function saveTransaction(draft: TransactionDraft): Promise<number> {
  return await invoke<number>("save_transaction", { draft });
}

/** Recent transactions, newest first. */
export async function listTransactions(limit: number): Promise<TxnSummary[]> {
  const rows = await invoke<RawTxnSummary[]>("list_transactions", { limit });
  return rows.map(brandSummary);
}

/** The needs-categorizing queue that `quick_add` feeds. */
export async function needsCategorizing(): Promise<TxnSummary[]> {
  const rows = await invoke<RawTxnSummary[]>("needs_categorizing");
  return rows.map(brandSummary);
}

export interface EntryContext {
  merchantName: string | null;
  locationName: string | null;
  taxRatePpm: number;
  defaultCategoryId: number | null;
  detailLevel: DetailLevel;
  recentMerchants: string[];
}

/**
 * What the app already remembers about a store.
 *
 * This is the store-memory feature and the reason the entry form is short:
 * naming a merchant recalls its usual category, its location and whether you
 * itemize there. Naming a new one is not an error and creates nothing — the
 * merchant row appears on save, so an abandoned entry leaves no debris.
 */
export async function entryContext(merchant: string | null): Promise<EntryContext> {
  return await invoke<EntryContext>("entry_context", { merchant });
}

export interface SubtotalRow {
  categoryId: number | null;
  subtotal: number;
  note: string | null;
}

/** A receipt entered the way it is read: a subtotal per category, no total. */
export interface SubtotalDraft {
  id: number | null;
  txnDate: string;
  merchant: string | null;
  location: string | null;
  rows: SubtotalRow[];
  notes: string | null;
  needsReview: boolean;
  rememberDefaults: boolean;
}

/** Reconcile a subtotal-entered receipt. Writes nothing; safe per keystroke. */
export async function previewSubtotals(draft: SubtotalDraft): Promise<Preview> {
  return brandPreview(await invoke<RawPreview>("preview_subtotals", { draft }));
}

export async function saveSubtotals(draft: SubtotalDraft): Promise<number> {
  return await invoke<number>("save_subtotals", { draft });
}

export interface Area {
  name: string;
  taxRatePpm: number;
}

/**
 * Named areas and the rate to prefill for each.
 *
 * Reference data, not database rows: starting points the user can override,
 * and a receipt's own tax always wins over any of them.
 */
export async function listAreas(): Promise<Area[]> {
  return await invoke<Area[]>("list_areas");
}

/** The home location, or null if none is set. */
export async function homeLocation(): Promise<Location | null> {
  return await invoke<Location | null>("home_location");
}

/** Name where the user lives and set its sales-tax rate. Asked on first run. */
export async function setHomeLocation(name: string, taxRatePpm: number): Promise<void> {
  await invoke("set_home_location", { name, taxRatePpm });
}

export interface Merchant {
  id: number;
  name: string;
  defaultCategoryId: number | null;
}

/** Every known store — seeded suggestions plus anywhere actually shopped. */
export async function listMerchants(): Promise<Merchant[]> {
  return await invoke<Merchant[]>("list_merchants");
}

export interface Location {
  id: number;
  name: string;
  taxRatePpm: number;
  isHome: boolean;
}

export async function listLocations(): Promise<Location[]> {
  return await invoke<Location[]>("list_locations");
}

/** Assign categories to a quick-added receipt and clear its review flag. */
export async function categorize(id: number, splits: SplitDraft[]): Promise<void> {
  await invoke("categorize", { id, splits });
}

interface RawBudgetRow {
  categoryId: number;
  categoryName: string;
  groupName: string;
  budgeted: number | null;
  perMonth: number | null;
  funded: number | null;
  spentCycle: number | null;
  overspentBy: number | null;
  spentThisMonth: number;
  funding: string;
  fundingBy: number;
}

export interface BudgetRow {
  categoryId: number;
  categoryName: string;
  groupName: string;
  budgeted: Cents | null;
  perMonth: Cents | null;
  funded: Cents | null;
  spentCycle: Cents | null;
  /** Non-zero is the real "over budget" signal. */
  overspentBy: Cents | null;
  spentThisMonth: Cents;
  /** `"ahead" | "on track" | "behind" | "none"` — a *funding* measure, which
   * is not the same question as overspending and disagrees with it in normal
   * use. Never render "behind" as "over budget". */
  funding: string;
  fundingBy: Cents;
}

function optionalCents(value: number | null): Cents | null {
  return value === null ? null : fromCore(value);
}

export async function budgetStatus(month: string): Promise<BudgetRow[]> {
  const rows = await invoke<RawBudgetRow[]>("budget_status", { month });
  return rows.map((r) => ({
    ...r,
    budgeted: optionalCents(r.budgeted),
    perMonth: optionalCents(r.perMonth),
    funded: optionalCents(r.funded),
    spentCycle: optionalCents(r.spentCycle),
    overspentBy: optionalCents(r.overspentBy),
    spentThisMonth: fromCore(r.spentThisMonth),
    fundingBy: fromCore(r.fundingBy),
  }));
}

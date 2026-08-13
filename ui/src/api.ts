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

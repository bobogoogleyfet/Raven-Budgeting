/**
 * M6's whole job: prove the toolchain end to end.
 *
 * This renders the transactions entered with `pacc add` in M3, through the
 * same `accounting-core` the CLI calls. When a row shows up here, the core is
 * genuinely UI-agnostic and the remaining frontends are a rendering exercise
 * rather than a re-implementation.
 *
 * Note what is absent: any arithmetic. The totals below are formatted, never
 * computed. See `money.ts`.
 */

import { listTransactions } from "./api.ts";
import { formatMoney } from "./money.ts";

const HOW_MANY = 50;

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  text?: string,
  className?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (text !== undefined) {
    node.textContent = text;
  }
  if (className !== undefined) {
    node.className = className;
  }
  return node;
}

async function render(): Promise<void> {
  const status = document.querySelector<HTMLParagraphElement>("#status");
  const table = document.querySelector<HTMLTableElement>("#transactions");
  const body = table?.querySelector("tbody");
  if (status === null || table === null || body == null) {
    return;
  }

  let rows;
  try {
    rows = await listTransactions(HOW_MANY);
  } catch (error) {
    // Surfacing the real message: at this stage the likely causes are a
    // missing database or a command that is not registered, and both are
    // things the message names precisely.
    status.textContent = `Could not load transactions: ${String(error)}`;
    return;
  }

  if (rows.length === 0) {
    status.textContent = "No receipts yet. Add one with `pacc add` and reload.";
    return;
  }

  for (const row of rows) {
    const tr = el("tr");
    tr.append(
      el("td", row.txnDate),
      el("td", row.merchantName ?? "—"),
      el("td", row.categories.join(", ")),
    );

    const total = el("td", formatMoney(row.total), "amount");
    if (row.taxIsEstimated) {
      // The estimate marker is not decoration. A computed tax that looks like
      // a figure off the receipt is the one thing the data model refuses to
      // let happen silently, so the UI must not undo that.
      total.append(el("span", "≈", "estimated"));
      total.title = "tax was estimated, not read off the receipt";
    }
    tr.append(total);

    if (row.needsReview) {
      tr.classList.add("needs-review");
      tr.title = "needs categorizing";
    }
    body.append(tr);
  }

  status.hidden = true;
  table.hidden = false;
}

void render();

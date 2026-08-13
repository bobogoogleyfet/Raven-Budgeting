/** The receipt list. Everything entered here or with `pacc add`. */

import { type TxnSummary, listTransactions } from "../api.ts";
import { clear, el } from "../dom.ts";
import { formatMoney } from "../money.ts";

const HOW_MANY = 200;

export function receiptsView(): HTMLElement {
  const root = el("section", { class: "view" });
  const status = el("p", { class: "hint", role: "status" }, ["Loading…"]);
  const table = el("table", { hidden: true });
  const body = el("tbody");

  table.append(
    el("thead", {}, [
      el("tr", {}, [
        el("th", {}, ["Date"]),
        el("th", {}, ["Store"]),
        el("th", {}, ["Categories"]),
        el("th", { class: "amount" }, ["Total"]),
      ]),
    ]),
    body,
  );

  function row(txn: TxnSummary): HTMLElement {
    const total = el("td", { class: "amount" }, [formatMoney(txn.total)]);
    if (txn.taxIsEstimated) {
      // Not decoration: the data model refuses to let a computed tax pass as
      // one read off a receipt, and the UI must not quietly undo that.
      total.append(
        el("span", { class: "estimated", title: "tax was estimated, not read off the receipt" }, [
          "≈",
        ]),
      );
    }
    const tr = el("tr", txn.needsReview ? { class: "needs-review", title: "needs categorizing" } : {}, [
      el("td", {}, [txn.txnDate]),
      el("td", {}, [txn.merchantName ?? "—"]),
      el("td", {}, [txn.categories.join(", ")]),
      total,
    ]);
    return tr;
  }

  root.append(el("h1", {}, ["Receipts"]), status, table);

  void listTransactions(HOW_MANY)
    .then((rows) => {
      if (rows.length === 0) {
        status.textContent = "No receipts yet. Add one on the Add tab.";
        return;
      }
      clear(body);
      for (const txn of rows) body.append(row(txn));
      status.hidden = true;
      table.hidden = false;
    })
    .catch((error: unknown) => {
      status.textContent = `Could not load receipts: ${String(error)}`;
    });

  return root;
}

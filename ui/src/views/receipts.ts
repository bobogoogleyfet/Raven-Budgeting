/** The receipt list. Everything entered here or with `pacc add`. */

import { type TxnSummary, deleteTransaction, listTransactions } from "../api.ts";
import { confirm } from "../dialog.ts";
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
        // Unlabelled: a header reading "Delete" over a column of delete
        // buttons says the same word twice, and this one is not a field of the
        // receipt.
        el("th", {}, []),
      ]),
    ]),
    body,
  );

  /**
   * The delete control: a click asks, a modal answers.
   *
   * The dialog names the receipt — its date, store and total — because "are you
   * sure?" is not a question anyone can answer about a row they may no longer
   * be looking at. There is no undo behind this, which is exactly why the
   * asking is worth a modal rather than something dismissible by accident.
   */
  function deleteCell(txn: TxnSummary, tr: HTMLElement): HTMLElement {
    const button = el("button", { type: "button", class: "link remove" }, ["Delete"]);

    button.addEventListener("click", () => {
      void confirm({
        question: "Delete this receipt?",
        detail: `${txn.txnDate} · ${txn.merchantName ?? "no store"} · ${formatMoney(txn.total)}`,
        confirmLabel: "Delete receipt",
      }).then((yes) => {
        if (!yes) return;
        button.disabled = true;
        return deleteTransaction(txn.id)
          .then(() => {
            // Removed from the page rather than reloading the list: a reload
            // would scroll the user back to the top of two hundred rows to
            // show them one that is gone.
            tr.remove();
            if (body.childElementCount === 0) {
              table.hidden = true;
              status.hidden = false;
              status.textContent = "No receipts yet. Add one on the Add tab.";
            }
          })
          .catch((error: unknown) => {
            button.disabled = false;
            status.hidden = false;
            status.textContent = `Could not delete that receipt: ${String(error)}`;
          });
      });
    });

    return el("td", {}, [button]);
  }

  function row(txn: TxnSummary): HTMLElement {
    const tr = el(
      "tr",
      txn.needsReview ? { class: "needs-review", title: "needs categorizing" } : {},
      [
        el("td", {}, [txn.txnDate]),
        el("td", {}, [txn.merchantName ?? "—"]),
        el("td", {}, [txn.categories.join(", ")]),
        el("td", { class: "amount" }, [formatMoney(txn.total)]),
      ],
    );
    tr.append(deleteCell(txn, tr));
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

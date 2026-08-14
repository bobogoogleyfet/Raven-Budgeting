/** The receipt list. Everything entered here or with `pacc add`. */

import { type TxnSummary, deleteTransaction, listTransactions } from "../api.ts";
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
   * The delete control: one click arms it, the second does it.
   *
   * Deliberately not a `confirm()` dialog. A modal is a different question in
   * a different place — "Are you sure?" with no receipt visible behind it — and
   * arming in place keeps the row, its date and its total on screen while the
   * user decides. It also costs nothing to change your mind: clicking anything
   * else, or the same button again, disarms it.
   *
   * There is no undo behind this, so the armed state says "Delete?" rather
   * than turning red and hoping.
   */
  function deleteCell(txn: TxnSummary, tr: HTMLElement): HTMLElement {
    const button = el("button", { type: "button", class: "link remove" }, ["Delete"]);
    let armed = false;

    const disarm = (): void => {
      armed = false;
      button.textContent = "Delete";
      button.classList.remove("armed");
    };

    button.addEventListener("click", () => {
      if (!armed) {
        armed = true;
        button.textContent = "Delete?";
        button.classList.add("armed");
        return;
      }
      button.disabled = true;
      void deleteTransaction(txn.id)
        .then(() => {
          // Removed from the page rather than reloading the list: a reload
          // would scroll the user back to the top of two hundred rows to show
          // them one that is gone.
          tr.remove();
          if (body.childElementCount === 0) {
            table.hidden = true;
            status.hidden = false;
            status.textContent = "No receipts yet. Add one on the Add tab.";
          }
        })
        .catch((error: unknown) => {
          button.disabled = false;
          disarm();
          status.hidden = false;
          status.textContent = `Could not delete that receipt: ${String(error)}`;
        });
    });

    // Wandering off means "no". Only a second click on this button is a yes.
    button.addEventListener("blur", disarm);
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

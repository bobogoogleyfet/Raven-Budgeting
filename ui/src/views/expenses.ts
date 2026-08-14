/** The expense list. Everything entered here or with `pacc add`. */

import {
  type Category,
  type Merchant,
  type TxnQuery,
  type TxnSort,
  type TxnSummary,
  deleteTransaction,
  listCategories,
  listMerchants,
  listTransactions,
} from "../api.ts";
import { confirmDialog } from "../dialog.ts";
import { clear, el } from "../dom.ts";
import { formatMoney } from "../money.ts";

const HOW_MANY = 200;

export function expensesView(): HTMLElement {
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
      void confirmDialog({
        question: "Delete this expense?",
        detail: `${txn.txnDate} · ${txn.merchantName ?? "no store"} · ${formatMoney(txn.total)}`,
        confirmLabel: "Delete Expense",
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
              status.textContent = "No expenses yet. Add one on the Add tab.";
            }
          })
          .catch((error: unknown) => {
            button.disabled = false;
            status.hidden = false;
            status.textContent = `Could not delete that expense: ${String(error)}`;
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

  /**
   * The narrowing and ordering controls.
   *
   * Plain `<select>`s rather than the filtering comboboxes the entry form uses:
   * those exist to let a user type three letters instead of scrolling two dozen
   * categories, which is worth it when every expense needs one. A filter is
   * touched rarely, and a native select is the control every user already knows
   * how to operate — including with a keyboard.
   */
  const categoryPicker = el("select", { id: "x-category" });
  const storePicker = el("select", { id: "x-store" });
  const sortPicker = el("select", { id: "x-sort" });

  const option = (value: string, label: string): HTMLOptionElement =>
    el("option", { value }, [label]);

  sortPicker.append(
    option("dateDesc", "Newest first"),
    option("dateAsc", "Oldest first"),
    option("totalDesc", "Largest first"),
    option("totalAsc", "Smallest first"),
  );

  function query(): TxnQuery {
    // An empty value is "no filter", not id 0 — every id is positive, so the
    // two can never be confused.
    const asId = (raw: string): number | null => (raw === "" ? null : Number(raw));
    return {
      limit: HOW_MANY,
      categoryId: asId(categoryPicker.value),
      merchantId: asId(storePicker.value),
      sort: sortPicker.value as TxnSort,
    };
  }

  function load(): void {
    void listTransactions(query())
      .then((rows) => {
        clear(body);
        if (rows.length === 0) {
          // Worded by case: an empty database and an over-narrow filter look
          // identical on screen and need different things done about them.
          const filtered = categoryPicker.value !== "" || storePicker.value !== "";
          status.textContent = filtered
            ? "No expenses match those filters."
            : "No expenses yet. Add one on the Add tab.";
          status.hidden = false;
          table.hidden = true;
          return;
        }
        for (const txn of rows) body.append(row(txn));
        status.hidden = true;
        table.hidden = false;
      })
      .catch((error: unknown) => {
        status.hidden = false;
        status.textContent = `Could not load expenses: ${String(error)}`;
      });
  }

  for (const picker of [categoryPicker, storePicker, sortPicker]) {
    picker.addEventListener("change", load);
  }

  root.append(
    el("h1", {}, ["Expenses"]),
    el("div", { class: "toolbar" }, [
      el("label", { for: "x-category" }, ["Category"]),
      categoryPicker,
      el("label", { for: "x-store" }, ["Store"]),
      storePicker,
      el("label", { for: "x-sort" }, ["Sort"]),
      sortPicker,
    ]),
    status,
    table,
  );

  // The filters are populated before the first load, so a list that arrives
  // fast is never briefly filterable by nothing.
  void Promise.all([listCategories(), listMerchants()])
    .then(([categories, stores]: [Category[], Merchant[]]) => {
      categoryPicker.append(
        option("", "All categories"),
        ...[...categories]
          .sort((a, b) => a.name.localeCompare(b.name))
          .map((c) => option(String(c.id), c.name)),
      );
      storePicker.append(
        option("", "All stores"),
        ...[...stores]
          .sort((a, b) => a.name.localeCompare(b.name))
          .map((m) => option(String(m.id), m.name)),
      );
    })
    .catch(() => {
      // A filter that failed to populate still leaves the list usable, and the
      // list's own error reporting is the one that matters.
      categoryPicker.append(option("", "All categories"));
      storePicker.append(option("", "All stores"));
    })
    .finally(load);

  return root;
}

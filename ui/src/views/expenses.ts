/** The expense list. Everything entered here or with `pacc add`. */

import {
  type TxnQuery,
  type TxnSort,
  type TxnSummary,
  deleteTransaction,
  filterValues,
  listTransactions,
} from "../api.ts";
import { confirmDialog } from "../dialog.ts";
import { clear, el } from "../dom.ts";
import { filterMenu } from "../filtermenu.ts";
import { caret, trash } from "../icons.ts";
import { formatMoney } from "../money.ts";

const HOW_MANY = 200;

/** Stands for expenses with no store, which is what quick-add leaves behind. */
const NO_STORE = "none";

export function expensesView(): HTMLElement {
  const root = el("section", { class: "view" });
  const status = el("p", { class: "hint", role: "status" }, ["Loading…"]);
  const table = el("table", { hidden: true });
  const body = el("tbody");

  /**
   * Newest first, and the caret in the Date header flips it.
   *
   * Only Date sorts. A list of expenses is read chronologically; the other
   * columns are what you narrow by, which is a different question and gets a
   * different control.
   */
  let sort: TxnSort = "dateDesc";
  // The title and the caret are one pill, so the whole header sorts rather
  // than an eight-pixel triangle being the only target.
  const dateSort = el("button", { type: "button", class: "sort-toggle" });
  const paintSort = (): void => {
    dateSort.replaceChildren(el("span", { class: "filter-label" }, ["Date"]));
    const icon = el("span", { class: "filter-icon" });
    icon.innerHTML = caret(sort === "dateDesc");
    dateSort.append(icon);
    dateSort.title = sort === "dateDesc" ? "Newest first" : "Oldest first";
  };
  dateSort.addEventListener("click", () => {
    sort = sort === "dateDesc" ? "dateAsc" : "dateDesc";
    paintSort();
    load();
  });
  paintSort();

  const storeFilter = filterMenu({ label: "Store", onChange: () => load() });
  const categoryFilter = filterMenu({ label: "Categories", onChange: () => load() });

  table.append(
    el("thead", {}, [
      el("tr", {}, [
        el("th", {}, [dateSort]),
        el("th", {}, [storeFilter.root]),
        el("th", {}, [categoryFilter.root]),
        el("th", { class: "amount" }, ["Total"]),
        // Unlabelled: a header reading "Delete" over a column of delete
        // buttons says the same word twice.
        el("th", {}, []),
      ]),
    ]),
    body,
  );

  /**
   * The delete control: a click asks, a modal answers.
   *
   * The dialog names the expense — its date, store and total — because "are you
   * sure?" is not a question anyone can answer about a row they may no longer
   * be looking at. There is no undo behind this, which is exactly why the
   * asking is worth a modal rather than something dismissible by accident.
   */
  function deleteCell(txn: TxnSummary, tr: HTMLElement): HTMLElement {
    // A trash can at the row's own text size: the label said "Delete" in every
    // row of a table whose other columns are the expense itself, which made a
    // column of the same word the widest thing on the screen.
    const button = el("button", {
      type: "button",
      class: "link icon-button",
      title: "Delete this expense",
      "aria-label": "Delete this expense",
    });
    button.innerHTML = trash();

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
            void loadMenus();
            if (body.childElementCount === 0) load();
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

  function showEmpty(filtered: boolean): void {
    clear(body);
    // Worded by case: an empty database and an over-narrow filter look
    // identical on screen and need different things done about them.
    status.textContent = filtered
      ? "No expenses match those filters."
      : "No expenses yet. Add one on the Add tab.";
    status.hidden = false;
    // The header stays, or there would be no filter left to widen.
    table.hidden = false;
  }

  function query(): TxnQuery | null {
    const stores = storeFilter.selected();
    const categories = categoryFilter.selected();
    // Everything hidden asks for nothing. The core reads an empty set as "no
    // filter", so that question is answered here rather than sent — a "Hide
    // all" that showed the whole table would be the opposite of itself.
    if (stores?.length === 0 || categories?.length === 0) return null;
    return {
      limit: HOW_MANY,
      merchantIds: stores?.filter((value) => value !== NO_STORE).map(Number) ?? null,
      includeNoMerchant: stores === null || stores.includes(NO_STORE),
      categoryIds: categories?.map(Number) ?? null,
      sort,
    };
  }

  function load(): void {
    const asked = query();
    if (asked === null) {
      showEmpty(true);
      return;
    }
    void listTransactions(asked)
      .then((rows) => {
        if (rows.length === 0) {
          showEmpty(storeFilter.selected() !== null || categoryFilter.selected() !== null);
          return;
        }
        clear(body);
        for (const txn of rows) body.append(row(txn));
        status.hidden = true;
        table.hidden = false;
      })
      .catch((error: unknown) => {
        status.hidden = false;
        status.textContent = `Could not load expenses: ${String(error)}`;
      });
  }

  // Under the table, not above it: it is what the list has to say when the
  // rows run out, and a message above the header pushes the whole table down
  // every time a filter matches nothing.
  root.append(el("h1", {}, ["Expenses"]), table, status);

  /**
   * Fill the menus from what the columns actually hold.
   *
   * Re-read after a delete as well as at startup: removing the last expense
   * from a shop should take that shop out of the menu, or the list offers a
   * filter that can only ever match nothing.
   */
  function loadMenus(): Promise<void> {
    return filterValues()
      .then((values) => {
        categoryFilter.setOptions(
          values.categories.map((c) => ({ value: String(c.id), label: c.name })),
        );
        storeFilter.setOptions([
          ...values.merchants.map((m) => ({ value: String(m.id), label: m.name })),
          // Quick-added expenses have no store, and without a box of their own
          // they would vanish the moment the filter was touched. Offered only
          // when there is one, so the menu never lists a row matching nothing.
          ...(values.anyWithoutMerchant ? [{ value: NO_STORE, label: "(No store)" }] : []),
        ]);
      })
      .catch(() => {
        // A menu that failed to fill still leaves the list usable, and the
        // list's own error reporting is the one that matters.
      });
  }

  // Filled before the first load, so a list that arrives fast is never briefly
  // filterable by nothing.
  void loadMenus().finally(load);

  return root;
}

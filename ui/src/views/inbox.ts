/**
 * The needs-categorizing queue.
 *
 * `pacc quick-add 42.18` records a total in the checkout line and files it
 * under Uncategorized so the decision can happen later. This is later. The
 * amounts are already on disk and are not re-entered here — only which
 * envelope they came out of, which is why each row needs one dropdown and
 * nothing else.
 */

import { type Category, categorize, listCategories, needsCategorizing } from "../api.ts";
import { clear, el } from "../dom.ts";
import { formatMoney } from "../money.ts";

export function inboxView(onChanged: () => void): HTMLElement {
  const root = el("section", { class: "view" });
  const status = el("p", { class: "hint", role: "status" }, ["Loading…"]);
  const list = el("div", { class: "inbox" });

  let categories: Category[] = [];

  async function refresh(): Promise<void> {
    const rows = await needsCategorizing();
    clear(list);

    if (rows.length === 0) {
      status.hidden = false;
      status.textContent = "Nothing waiting. Quick-added expenses show up here.";
      return;
    }
    status.hidden = true;

    for (const txn of rows) {
      const picker = el("select", { class: "category" });
      picker.append(el("option", { value: "" }, ["Choose a category…"]));
      for (const c of categories) {
        picker.append(el("option", { value: String(c.id) }, [`${c.groupName} / ${c.name}`]));
      }

      const file = el("button", { type: "button", class: "primary small" }, ["File it"]);
      file.disabled = true;
      picker.addEventListener("change", () => {
        file.disabled = picker.value === "";
      });

      const problem = el("span", { class: "blocker" });

      file.addEventListener("click", () => {
        file.disabled = true;
        clear(problem);
        // One category taking the whole receipt: `remainder` rather than the
        // total, so the core assigns the exact stored amount and the UI never
        // has to restate a figure it might get wrong.
        void categorize(txn.id, [
          { categoryId: Number(picker.value), amount: { kind: "remainder" }, note: null },
        ])
          .then(() => refresh())
          .then(() => onChanged())
          .catch((error: unknown) => {
            problem.textContent = String(error);
            file.disabled = false;
          });
      });

      list.append(
        el("div", { class: "inbox-row" }, [
          el("span", { class: "inbox-date" }, [txn.txnDate]),
          el("span", {}, [txn.merchantName ?? "—"]),
          el("span", { class: "amount" }, [formatMoney(txn.total)]),
          picker,
          file,
          problem,
        ]),
      );
    }
  }

  root.append(el("h1", {}, ["Inbox"]), status, list);

  void (async () => {
    try {
      categories = await listCategories();
      await refresh();
    } catch (error) {
      status.hidden = false;
      status.textContent = `Could not load the inbox: ${String(error)}`;
    }
  })();

  return root;
}

/**
 * Four screens and a tab bar.
 *
 * Views are rebuilt on switch rather than cached. The data behind them changes
 * whenever a receipt is saved or filed, and a stale Receipts tab that silently
 * omits the row you just entered is worse than a rebuild nobody can perceive
 * against a local SQLite file.
 */

import { need } from "./dom.ts";
import { addView } from "./views/add.ts";
import { budgetView } from "./views/budget.ts";
import { inboxView } from "./views/inbox.ts";
import { receiptsView } from "./views/receipts.ts";

type TabName = "add" | "receipts" | "inbox" | "budget";

const main = need("#main");
const tabs = need("#tabs");

let current: TabName = "add";

function build(tab: TabName): HTMLElement {
  switch (tab) {
    case "add":
      // Saving a receipt makes the other two tabs stale, so re-render on the
      // way in rather than tracking what changed.
      return addView(() => undefined);
    case "receipts":
      return receiptsView();
    case "inbox":
      return inboxView(() => undefined);
    case "budget":
      return budgetView();
  }
}

function show(tab: TabName): void {
  current = tab;
  for (const button of tabs.querySelectorAll("button")) {
    button.classList.toggle("active", button.dataset["tab"] === tab);
  }
  main.replaceChildren(build(tab));
}

tabs.addEventListener("click", (event) => {
  const target = event.target;
  if (!(target instanceof HTMLButtonElement)) return;
  const tab = target.dataset["tab"];
  if (tab === undefined || tab === current) return;
  show(tab as TabName);
});

show("add");

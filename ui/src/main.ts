/**
 * Four screens and a tab bar.
 *
 * Views are rebuilt on switch rather than cached. The data behind them changes
 * whenever an expense is saved or filed, and a stale Expenses tab that silently
 * omits the row you just entered is worse than a rebuild nobody can perceive
 * against a local SQLite file.
 */

import { homeLocation } from "./api.ts";
import { need } from "./dom.ts";
import { addView } from "./views/add.ts";
import { budgetView } from "./views/budget.ts";
import { inboxView } from "./views/inbox.ts";
import { expensesView } from "./views/expenses.ts";
import { settingsView } from "./views/settings.ts";
import { setupView } from "./views/setup.ts";

type TabName = "add" | "expenses" | "inbox" | "budget" | "settings";

const main = need("#main");
const tabs = need("#tabs");

let current: TabName = "add";

function build(tab: TabName): HTMLElement {
  switch (tab) {
    case "add":
      // Saving an expense makes the other two tabs stale, so re-render on the
      // way in rather than tracking what changed.
      return addView(() => undefined);
    case "expenses":
      return expensesView();
    case "inbox":
      return inboxView(() => undefined);
    case "budget":
      return budgetView();
    case "settings":
      return settingsView();
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

/**
 * First run asks where the user lives before anything else.
 *
 * Detected by the home location still carrying a 0% rate, which is what the
 * seed installs: no rate means every receipt would record $0.00 of tax, and
 * the app cannot guess the right one. A user who genuinely lives somewhere
 * without sales tax types 0 and is not asked again.
 */
void (async () => {
  let home = null;
  try {
    home = await homeLocation();
  } catch {
    // A failure here is not worth blocking entry over; the Add tab will
    // surface the real error when it calls the same backend.
  }

  if (home === null || home.taxRatePpm === 0) {
    tabs.hidden = true;
    main.replaceChildren(
      setupView(() => {
        tabs.hidden = false;
        show("add");
      }),
    );
    return;
  }
  show("add");
})();

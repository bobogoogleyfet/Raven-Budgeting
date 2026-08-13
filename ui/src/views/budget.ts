/**
 * The budget screen.
 *
 * # Funding and overspending are different columns on purpose
 *
 * They answer different questions and disagree in normal use. A $400/month
 * grocery envelope with $400 spent is *behind on funding* — the schedule says
 * that money should be banked and it is not — while being exactly on budget on
 * spending. Rendering "behind" as "over budget" would flag every well-run
 * category and train the user to ignore the screen.
 *
 * So `funding` is shown as its own column, and the over-budget highlight keys
 * strictly on `overspentBy`, which the core sets only when spending has
 * actually passed the cycle's budget.
 */

import { type BudgetRow, budgetStatus } from "../api.ts";
import { clear, el, thisMonth } from "../dom.ts";
import { formatMoney } from "../money.ts";

export function budgetView(): HTMLElement {
  const root = el("section", { class: "view" });
  const month = el("input", { type: "month", id: "b-month", value: thisMonth() });
  const status = el("p", { class: "hint", role: "status" }, ["Loading…"]);
  const table = el("table", { hidden: true, class: "budget" });
  const body = el("tbody");
  const unbudgeted = el("p", { class: "hint" });

  table.append(
    el("thead", {}, [
      el("tr", {}, [
        el("th", {}, ["Category"]),
        el("th", { class: "amount" }, ["Per month"]),
        el("th", { class: "amount" }, ["Funded"]),
        el("th", { class: "amount" }, ["Spent (cycle)"]),
        el("th", {}, ["Funding"]),
        el("th", { class: "amount" }, ["Over"]),
      ]),
    ]),
    body,
  );

  function row(r: BudgetRow): HTMLElement {
    const over = r.overspentBy !== null && r.overspentBy !== 0;
    const funding =
      r.funding === "none"
        ? "—"
        : r.funding === "on track"
          ? "on track"
          : `${r.funding} ${formatMoney(r.fundingBy)}`;

    return el("tr", over ? { class: "over" } : {}, [
      el("td", {}, [`${r.groupName} / ${r.categoryName}`]),
      el("td", { class: "amount" }, [r.perMonth === null ? "—" : formatMoney(r.perMonth)]),
      el("td", { class: "amount" }, [r.funded === null ? "—" : formatMoney(r.funded)]),
      el("td", { class: "amount" }, [r.spentCycle === null ? "—" : formatMoney(r.spentCycle)]),
      el("td", { class: `funding ${r.funding.replace(" ", "-")}` }, [funding]),
      el("td", { class: "amount" }, [over && r.overspentBy !== null ? formatMoney(r.overspentBy) : "—"]),
    ]);
  }

  async function refresh(): Promise<void> {
    try {
      const rows = await budgetStatus(month.value);
      const budgeted = rows.filter((r) => r.perMonth !== null);
      clear(body);
      for (const r of budgeted) body.append(row(r));

      const without = rows.length - budgeted.length;
      unbudgeted.textContent =
        without === 0 ? "" : `${without} categories have no budget — set one with \`pacc budget set\`.`;

      if (budgeted.length === 0) {
        status.hidden = false;
        status.textContent = "No budgets set for this month.";
        table.hidden = true;
        return;
      }
      status.hidden = true;
      table.hidden = false;
    } catch (error) {
      status.hidden = false;
      status.textContent = `Could not load budgets: ${String(error)}`;
    }
  }

  month.addEventListener("change", () => void refresh());

  root.append(
    el("h1", {}, ["Budget"]),
    el("div", { class: "toolbar" }, [el("label", { for: "b-month" }, ["Month"]), month]),
    status,
    table,
    unbudgeted,
    el("p", { class: "hint footnote" }, [
      "Funding asks whether the money will be there when the bill lands; Over asks whether more has been spent than this cycle's budget. They disagree by design — an envelope spent exactly to plan is behind on funding and fine on spending.",
    ]),
  );

  void refresh();
  return root;
}

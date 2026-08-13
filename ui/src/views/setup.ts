/**
 * The first-run prompt: where do you live, and what is the sales tax there?
 *
 * Asked once, up front, because it is the one setting the app cannot guess and
 * cannot work without. Every receipt that does not name its own location
 * inherits this rate, so leaving it at zero means quietly recording $0.00 of
 * tax on every purchase — wrong in a way that no error message surfaces and
 * that only shows up months later when the totals do not match the bank.
 *
 * The seed deliberately ships a 0% "Home" rather than guessing a rate: a wrong
 * prefilled rate produces plausible figures that disagree with the receipt,
 * which is worse than an obviously missing one. This is where that gets fixed.
 */

import { setHomeLocation } from "../api.ts";
import { el } from "../dom.ts";

/** `"7.75"` → `77500` ppm. `null` if it is not a usable rate. */
export function parseRatePpm(input: string): number | null {
  const cleaned = input.trim().replace(/%$/, "").trim();
  if (!/^\d*\.?\d+$/.test(cleaned)) return null;
  const percent = Number(cleaned);
  if (!Number.isFinite(percent) || percent < 0 || percent > 100) return null;
  // Parts per million, so 9.4375% is exactly 94375 rather than a rounded
  // 9.44%. Rounding at the fourth decimal is below a hundredth of a cent on
  // any real receipt and keeps the value an integer.
  return Math.round(percent * 10_000);
}

export function setupView(onDone: () => void): HTMLElement {
  const root = el("section", { class: "view setup" });

  const name = el("input", { type: "text", id: "s-name", placeholder: "Highland, UT" });
  const rate = el("input", {
    type: "text",
    id: "s-rate",
    inputmode: "decimal",
    placeholder: "7.75",
  });
  const problem = el("p", { class: "blocker" });
  const go = el("button", { type: "button", class: "primary" }, ["Save and start"]);

  function submit(): void {
    problem.textContent = "";
    const where = name.value.trim();
    const ppm = parseRatePpm(rate.value);

    if (where === "") {
      problem.textContent = "Name the place you shop most — a town is enough.";
      return;
    }
    if (ppm === null) {
      problem.textContent = "Enter the sales-tax rate as a percentage, like 7.75.";
      return;
    }

    go.disabled = true;
    void setHomeLocation(where, ppm)
      .then(() => onDone())
      .catch((error: unknown) => {
        problem.textContent = String(error);
        go.disabled = false;
      });
  }

  for (const field of [name, rate]) {
    field.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        submit();
      }
    });
  }
  go.addEventListener("click", submit);

  root.append(
    el("h1", {}, ["Where do you shop?"]),
    el("p", { class: "hint" }, [
      "Sales tax is worked out from where a store is, so receipts only need their subtotals. ",
      "This is the default for anywhere you have not named specifically.",
    ]),
    el("div", { class: "form" }, [
      el("label", { for: "s-name" }, ["Home area"]),
      name,
      el("label", { for: "s-rate" }, ["Sales tax"]),
      rate,
    ]),
    el("p", { class: "hint footnote" }, [
      "You can change this any time, and a receipt's own tax always wins over the estimate.",
    ]),
    el("div", { class: "actions" }, [go, problem]),
  );

  setTimeout(() => name.focus(), 0);
  return root;
}

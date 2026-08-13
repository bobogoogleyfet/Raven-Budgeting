/**
 * The first-run prompt: where do you shop?
 *
 * Asked once, up front, because it is the one setting the app cannot guess and
 * cannot work without. Every receipt that does not name its own location
 * inherits this rate, so leaving it unset means quietly recording $0.00 of tax
 * on every purchase — wrong in a way no error message surfaces, and that only
 * shows up months later when the totals do not match the bank.
 *
 * # The rate comes from the area, not from the user
 *
 * Picking a place fills in its sales-tax rate. Nobody should have to look up a
 * combined state-plus-county-plus-city rate before entering a first receipt.
 *
 * The rate is still shown rather than hidden, because it is an **estimate**:
 * these rates change, and a single per-area number cannot express Utah's
 * reduced rate on unprepared food. Showing it is what lets a user notice it is
 * wrong. An area the list does not know asks for the rate directly, since
 * there is nothing to derive it from.
 */

import { type Area, listAreas, setHomeLocation } from "../api.ts";
import { combobox } from "../combobox.ts";
import { el } from "../dom.ts";
import { formatRatePpm } from "../money.ts";

/** `"7.75"` → `77500` ppm. `null` if it is not a usable rate. */
export function parseRatePpm(input: string): number | null {
  const cleaned = input.trim().replace(/%$/, "").trim();
  if (!/^\d*\.?\d+$/.test(cleaned)) return null;
  const percent = Number(cleaned);
  if (!Number.isFinite(percent) || percent < 0 || percent > 100) return null;
  // Parts per million, so 9.4375% is exactly 94375 rather than a rounded
  // 9.44%. Rounding at the fourth decimal is far below a cent on any real
  // receipt and keeps the value an integer.
  return Math.round(percent * 10_000);
}

export function setupView(onDone: () => void): HTMLElement {
  const root = el("section", { class: "view setup" });

  let areas: Area[] = [];

  const area = combobox({
    id: "s-area",
    placeholder: "Start typing your city…",
    onCommit: () => refreshRate(),
  });

  // Shown for a known area; replaced by an input when the area is not listed.
  const derived = el("p", { class: "derived-rate" });
  const rateInput = el("input", {
    type: "text",
    id: "s-rate",
    inputmode: "decimal",
    placeholder: "7.25",
  });
  const rateRow = el("div", { class: "rate-row" }, [derived]);

  const problem = el("p", { class: "blocker" });
  const go = el("button", { type: "button", class: "primary" }, ["Save and start"]);

  function known(): Area | undefined {
    const name = area.value().toLowerCase();
    return areas.find((a) => a.name.toLowerCase() === name);
  }

  function refreshRate(): void {
    const match = known();
    rateRow.replaceChildren();
    if (area.value() === "") {
      derived.textContent = "";
      rateRow.append(derived);
      return;
    }
    if (match === undefined) {
      // Nothing to derive from, so ask. This is also the escape hatch for
      // anywhere outside the shipped list.
      rateRow.append(
        el("span", { class: "hint" }, ["Not in the list — what is the sales tax there? "]),
        rateInput,
        el("span", { class: "hint" }, ["%"]),
      );
      return;
    }
    derived.textContent = `Sales tax ${formatRatePpm(match.taxRatePpm)}`;
    rateRow.append(
      derived,
      el("span", { class: "hint" }, [" — an estimate; a receipt's own tax always wins."]),
    );
  }

  function submit(): void {
    problem.textContent = "";
    const where = area.value();
    if (where === "") {
      problem.textContent = "Pick the area you shop in most, or type it in.";
      return;
    }

    const match = known();
    const ppm = match !== undefined ? match.taxRatePpm : parseRatePpm(rateInput.value);
    if (ppm === null) {
      problem.textContent = "Enter the sales-tax rate for that area, like 7.25.";
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

  for (const field of [area.input, rateInput]) {
    field.addEventListener("keydown", (event) => {
      // Enter inside the combobox picks a row first; only submit once the
      // list has closed, or picking a suggestion would also save.
      if (event.key === "Enter" && field === rateInput) {
        event.preventDefault();
        submit();
      }
    });
  }
  area.input.addEventListener("change", () => refreshRate());
  go.addEventListener("click", submit);

  root.append(
    el("h1", {}, ["Where do you shop?"]),
    el("p", { class: "hint" }, [
      "Sales tax is worked out from where a store is, so receipts only need their subtotals. ",
      "This is the default for anywhere you have not named specifically.",
    ]),
    el("div", { class: "form" }, [el("label", { for: "s-area" }, ["Home area"]), area.root]),
    rateRow,
    el("div", { class: "actions" }, [go, problem]),
    el("p", { class: "hint footnote" }, [
      "You can change this any time. Rates here are starting points, not gospel — they change, ",
      "and Utah taxes unprepared food at a lower rate than everything else.",
    ]),
  );

  void listAreas()
    .then((list) => {
      areas = list;
      area.setOptions(list.map((a) => a.name));
    })
    .catch((error: unknown) => {
      problem.textContent = `Could not load the area list: ${String(error)}`;
    });

  setTimeout(() => area.input.focus(), 0);
  return root;
}

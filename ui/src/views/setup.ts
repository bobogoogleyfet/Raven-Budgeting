/**
 * The first-run prompt: where do you shop?
 *
 * Asked once, up front, because it is the one setting the app cannot guess and
 * cannot work without. Every receipt that does not name its own location
 * inherits this rate, so leaving it unset means quietly recording $0.00 of tax
 * on every purchase — wrong in a way no error message surfaces, and that only
 * shows up months later when the totals do not match the bank.
 *
 * # The rate comes from the state, not from the user
 *
 * Picking a place fills in its sales-tax rate. Nobody should have to look up a
 * combined state-plus-county-plus-city rate before entering a first receipt.
 *
 * The list is every taxing jurisdiction Utah publishes, refreshed from the Tax
 * Commission's own quarterly file. That is why the app is Utah-only for now:
 * the rates are one state's publication, and the other forty-nine would each
 * need their own publisher, schedule and file format rather than a bigger list.
 *
 * The rate is still shown rather than hidden, because it is an **estimate**:
 * rates change on the first day of every quarter, and showing one is what lets
 * a user notice it is wrong. So is the quarter it came from — a table that
 * quietly stopped updating looks exactly like a current one otherwise.
 *
 * An area the list does not know asks for the rate directly, since there is
 * nothing to derive it from. Outside Utah that is every area, which is the
 * honest form of "not supported yet": the app still works, it just cannot fill
 * the number in.
 *
 * An area brings a second figure where the state has one: Utah taxes
 * unprepared food at a combined 3% rather than ~7.25%, which on a $7.28
 * grocery subtotal is the difference between $0.22 of tax and $0.53. It is
 * shown alongside the general rate for the same reason that one is, and a
 * hand-typed area carries no food rate — there is no second number to derive,
 * and inventing one would tax groceries at a figure nobody supplied.
 */

import { type Area, listAreas, rateRelease, setHomeLocation } from "../api.ts";
import { combobox } from "../combobox.ts";
import { el } from "../dom.ts";
import { rateSummary } from "../money.ts";

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

/**
 * The line naming which publication the rates came from.
 *
 * Says "superseded" rather than "out of date" when the quarter has turned: the
 * figures are the state's last published ones, not junk, and the app is still
 * usable on them. What the user needs to know is that a newer set exists and
 * this machine has not managed to fetch it — which on a laptop that has been
 * offline is information, not an error.
 */
export function releaseLine(release: { quarter: string; current: boolean } | null): string {
  if (release === null) return "";
  const source = `Rates from Utah's ${release.quarter} publication`;
  return release.current
    ? `${source}.`
    : `${source} — superseded; a newer one has not downloaded yet.`;
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

  const provenance = el("p", { class: "hint footnote" });
  const problem = el("p", { class: "blocker" });
  const go = el("button", { type: "button", class: "primary" }, ["Save and Start"]);

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
      // Nothing to derive from, so ask. Outside Utah this is every area, and
      // it is the escape hatch that keeps "not supported yet" from meaning
      // "unusable".
      rateRow.append(
        el("span", { class: "hint" }, ["Not in Utah's list — what is the sales tax there? "]),
        rateInput,
        el("span", { class: "hint" }, ["%"]),
      );
      return;
    }
    derived.textContent = `Sales tax ${rateSummary(match.taxRatePpm, match.foodRatePpm)}`;
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
    // A hand-typed area gets null for both: the user moved the choice off the
    // list, so there is no jurisdiction to follow, and carrying Utah's 3% over
    // to Boise would undertax every grocery receipt entered there.
    const foodPpm = match?.foodRatePpm ?? null;
    const code = match?.code ?? null;

    go.disabled = true;
    void setHomeLocation(where, code, ppm, foodPpm)
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
      "Sales tax is worked out from where a store is, so expenses only need their subtotals. ",
      "This is the default for anywhere you have not named specifically.",
    ]),
    el("div", { class: "form" }, [el("label", { for: "s-area" }, ["Home area"]), area.root]),
    rateRow,
    el("div", { class: "actions" }, [go, problem]),
    provenance,
    el("p", { class: "hint footnote" }, [
      "You can change this any time. Rates here are starting points, not gospel — they change. ",
      "Groceries are taxed at the lower rate where the state has one; a restaurant meal is not, ",
      "because prepared food is taxed in full.",
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

  // Not awaited with the list: a missing provenance line is cosmetic, and the
  // picker must not wait on it to become usable.
  void rateRelease()
    .then((release) => {
      provenance.textContent = releaseLine(release);
    })
    .catch(() => {
      provenance.textContent = "";
    });

  setTimeout(() => area.input.focus(), 0);
  return root;
}

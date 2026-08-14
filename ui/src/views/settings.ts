/**
 * Settings: the handful of answers the app asked for once and then relied on.
 *
 * Today that is the home area — the location every expense inherits unless it
 * names another one. It was asked at first run and there was no way back to it,
 * so a user who moved, or who picked the wrong town in a list of four hundred,
 * had no way to correct the rate every later expense was estimated with.
 *
 * The same picker as first run, deliberately. It is the same question, and a
 * second arrangement of it would be a second thing to keep right.
 */

import { type Area, listAreas, rateRelease, setHomeLocation } from "../api.ts";
import { combobox } from "../combobox.ts";
import { clear, el } from "../dom.ts";
import { rateSummary } from "../money.ts";
import { parseRatePpm, releaseLine } from "./setup.ts";

export function settingsView(): HTMLElement {
  const root = el("section", { class: "view" });
  let areas: Area[] = [];

  const area = combobox({
    id: "s-home",
    placeholder: "Select a Location",
    onCommit: () => refreshRate(),
  });

  const derived = el("p", { class: "derived-rate" });
  const rateInput = el("input", {
    type: "text",
    id: "s-home-rate",
    inputmode: "decimal",
    placeholder: "7.45",
  });
  const rateRow = el("div", { class: "rate-row" }, [derived]);
  const provenance = el("p", { class: "hint footnote" });
  const feedback = el("p", { class: "hint", role: "status" });
  const save = el("button", { type: "button", class: "primary" }, ["Save Location"]);

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
      rateRow.append(
        el("span", { class: "hint" }, ["Not in Utah's list — what is the sales tax there? "]),
        rateInput,
        el("span", { class: "hint" }, ["%"]),
      );
      return;
    }
    derived.textContent = `Sales tax ${rateSummary(match.taxRatePpm, match.foodRatePpm)}`;
    rateRow.append(derived);
  }

  function submit(): void {
    clear(feedback);
    const where = area.value();
    if (where === "") {
      feedback.textContent = "Pick the area you shop in most, or type it in.";
      return;
    }
    const match = known();
    const ppm = match !== undefined ? match.taxRatePpm : parseRatePpm(rateInput.value);
    if (ppm === null) {
      feedback.textContent = "Enter the sales-tax rate for that area, like 7.45.";
      return;
    }

    save.disabled = true;
    // Renames the existing home rather than adding a second one, so every
    // expense already filed against it keeps pointing at the same row — and
    // keeps the rate it was entered with, which lives on the transaction.
    void setHomeLocation(where, match?.code ?? null, ppm, match?.foodRatePpm ?? null)
      .then(() => {
        feedback.textContent = `Home is now ${where}.`;
      })
      .catch((error: unknown) => {
        feedback.textContent = String(error);
      })
      .finally(() => {
        save.disabled = false;
      });
  }

  area.input.addEventListener("change", () => refreshRate());
  save.addEventListener("click", submit);

  root.append(
    el("h1", {}, ["Settings"]),
    el("h2", {}, ["Home area"]),
    el("p", { class: "hint" }, [
      "Every expense is taxed at this area's rate unless it names another location. ",
      "Changing it affects what the next expense estimates, never what is already filed.",
    ]),
    el("div", { class: "form" }, [el("label", { for: "s-home" }, ["Home area"]), area.root]),
    rateRow,
    el("div", { class: "actions" }, [save, feedback]),
    provenance,
  );

  void listAreas()
    .then((list) => {
      areas = list;
      area.setOptions(list.map((a) => a.name));
    })
    .catch((error: unknown) => {
      feedback.textContent = `Could not load the area list: ${String(error)}`;
    });

  // Shown for the same reason the rate is: a table that quietly stopped
  // updating looks exactly like a current one.
  void rateRelease()
    .then((release) => {
      provenance.textContent = releaseLine(release);
    })
    .catch(() => {
      provenance.textContent = "";
    });

  return root;
}

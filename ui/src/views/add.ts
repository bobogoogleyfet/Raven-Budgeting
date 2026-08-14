/**
 * The receipt entry form.
 *
 * # You type what the receipt shows, and nothing else
 *
 * A receipt gives you a subtotal per category. It also gives you a tax line
 * and a total, but both of those are *derivable* — and asking for a number the
 * app can work out means the two can disagree, which is then the user's problem
 * to adjudicate. So this form asks for the parts only: tax comes from the
 * location's sales-tax rate, and the total is the sum plus that tax.
 *
 * # The numbers are never this file's opinion
 *
 * Every change sends the rows to `preview_subtotals`, which reconciles the
 * amounts and apportions the tax in Rust and returns exactly what `save` would
 * write. The confirmation dialog renders that answer verbatim, so the figures
 * on screen are not the UI's reading of the receipt — they *are* what will be
 * stored.
 *
 * That is also why this file contains no arithmetic. It never adds two
 * subtotals and never applies a rate; it asks, and it renders.
 */

import {
  type Category,
  type Preview,
  type SubtotalDraft,
  type SubtotalRow,
  entryContext,
  listCategories,
  ensureLocation,
  listAreas,
  listLocations,
  listMerchants,
  previewSubtotals,
  saveSubtotals,
} from "../api.ts";
import { combobox } from "../combobox.ts";
import { dateField } from "../datefield.ts";
import { clear, debounce, el, today } from "../dom.ts";
import { categoryRateLabel, parseCents } from "../money.ts";
import { type ReviewResult, linesFrom, reviewDialog } from "./review.ts";

/** One entered line: a category and what it came to before tax. */
interface RowState {
  categoryId: number | null;
  subtotalText: string;
}

export function addView(onSaved: () => void): HTMLElement {
  const root = el("section", { class: "view" });

  let categories: Category[] = [];
  let rows: RowState[] = [{ categoryId: null, subtotalText: "" }];
  let locationName: string | null = null;
  let categoryRates = new Map<number, number>();
  /**
   * One per rendered row: re-reads its rate from `categoryRates` and repaints
   * the label in place.
   *
   * Kept instead of calling `renderRows()` when the context changes, because
   * re-rendering replaces the very `<input>` the user is mid-click on: the
   * browser hands focus to an element that no longer exists, the click is
   * lost, and the field needs clicking twice. That is a real bug this form
   * had, and it is exactly what a rebuild costs.
   */
  let rateLabels: (() => void)[] = [];
  /**
   * One per rendered row: re-offers the categories nobody else has taken.
   *
   * Filing two rows against one category is not a receipt with two lines, it
   * is one line entered twice — the splits would be indistinguishable in every
   * report, and the fix is to add the amounts together. Removing the taken
   * ones from the list says so before the user can do it.
   */
  let optionRefreshers: (() => void)[] = [];
  /** The merchant whose context is loaded, so leaving the field is free. */
  let loadedFor: string | null | undefined;
  /** The location this form filled in last, to tell it from a user's choice. */
  let lastDefaultPlace = "";
  let favourites: string[] = [];
  let saving = false;

  const merchant = combobox({
    id: "f-merchant",
    placeholder: "Select a Store",
    onCommit: (name) => {
      void loadContext(name === "" ? null : name).then(() => schedulePreview());
    },
  });

  const date = dateField({
    id: "f-date",
    onChange: () => schedulePreview(),
    onEnter: () => merchant.input.focus(),
  });
  date.setValue(today());

  /**
   * Where this was bought, which is what decides the tax.
   *
   * Optional in the sense that it is already answered: it starts at the store's
   * remembered location, else home, and most expenses never touch it. It is
   * here for the ones that do — a trip to another town is taxed at that town's
   * rate, and without this the receipt would quietly record the home rate.
   */
  const place = combobox({
    id: "f-place",
    placeholder: "Select a Location",
    onCommit: (name) => {
      // Created here rather than on save, because the preview has to be able
      // to name it: the core refuses a location it does not know, and showing
      // the home rate instead would put a wrong number on screen. Picking from
      // the list is a deliberate act, unlike the half-typed merchant names the
      // form is careful not to persist.
      if (name === "") {
        schedulePreview();
        return;
      }
      void ensureLocation(name)
        .then(() => schedulePreview())
        .catch(() => schedulePreview());
    },
  });

  const note = el("input", { type: "text", id: "f-note", placeholder: "optional" });
  const chips = el("div", { class: "chips" });
  const rowsBox = el("div", { class: "splits" });
  const addRow = el("button", { type: "button", class: "primary add-category" }, [
    "+ Add Category",
  ]);
  const save = el("button", { type: "button", class: "primary" }, ["Save Expense"]);
  const feedback = el("p", { class: "hint", role: "status" });

  // --- draft ----------------------------------------------------------------

  function draft(overrides?: SubtotalRow[]): SubtotalDraft {
    const entered: SubtotalRow[] = [];
    if (overrides === undefined) {
      for (const row of rows) {
        const cents = parseCents(row.subtotalText);
        // A row still being typed is left out rather than sent as zero: a
        // $0.00 line would claim the user meant to file nothing under that
        // category.
        if (cents !== null) {
          entered.push({ categoryId: row.categoryId, subtotal: cents, tax: null, note: null });
        }
      }
    } else {
      entered.push(...overrides);
    }
    const noteText = note.value.trim();
    return {
      id: null,
      txnDate: date.value(),
      merchant: merchant.value() || null,
      location: place.value() || locationName,
      rows: entered,
      notes: noteText === "" ? null : noteText,
      needsReview: false,
      rememberDefaults: true,
    };
  }

  // --- rendering ------------------------------------------------------------

  /** The categories still free, plus whichever this row already holds. */
  function availableTo(state: RowState): Category[] {
    const taken = new Set(
      rows.filter((r) => r !== state && r.categoryId !== null).map((r) => r.categoryId),
    );
    // Alphabetical, not the seed's grouping order. The list is filtered by
    // typing, so its only job unfiltered is to be scannable — and the one
    // order a reader can predict is the alphabet.
    return categories
      .filter((c) => !taken.has(c.id))
      .sort((a, b) => a.name.localeCompare(b.name));
  }

  function renderRows(): void {
    clear(rowsBox);
    rateLabels = [];
    optionRefreshers = [];
    rows.forEach((state, index) => {
      // The same filtering combobox as the store field: two dozen categories
      // is more than a dropdown is comfortable to scan, and typing three
      // letters beats scrolling on every receipt.
      const picker = combobox({
        placeholder: "Select a Category",
        onCommit: (name) => {
          const match = categories.find((c) => c.name.toLowerCase() === name.toLowerCase());
          state.categoryId = match?.id ?? null;
          showRate();
          // Every row, not just this one: choosing Groceries here has to take
          // Groceries out of the others' lists. In place rather than by
          // re-rendering, which would destroy the field being tabbed into.
          for (const refresh of optionRefreshers) refresh();
          schedulePreview();
        },
      });
      const refreshOptions = (): void => {
        picker.setOptions(availableTo(state).map((c) => c.name));
      };
      refreshOptions();
      optionRefreshers.push(refreshOptions);
      const chosen = categories.find((c) => c.id === state.categoryId);
      if (chosen !== undefined) picker.input.value = chosen.name;

      const amount = el("input", {
        type: "text",
        class: "amount-in",
        inputmode: "decimal",
        value: state.subtotalText,
        placeholder: "0.00",
      });
      amount.addEventListener("input", () => {
        state.subtotalText = amount.value;
        schedulePreview();
      });

      // The rate belongs beside the category that determines it. Shown as soon
      // as one is picked, before any amount is typed: "what will this be taxed
      // at" is a question about the category alone.
      const rate = el("p", { class: "hint row-rate" });
      const showRate = (): void => {
        const ppm = state.categoryId === null ? undefined : categoryRates.get(state.categoryId);
        rate.textContent = ppm === undefined ? "" : categoryRateLabel(ppm);
      };
      showRate();
      rateLabels.push(showRate);
      picker.input.addEventListener("change", showRate);

      const row = el("div", { class: "split-row" }, [
        el("div", { class: "split-cell" }, [picker.root, rate]),
        amount,
      ]);
      if (rows.length > 1) {
        const remove = el("button", { type: "button", class: "link remove" }, ["×"]);
        remove.addEventListener("click", () => {
          rows.splice(index, 1);
          renderRows();
          schedulePreview();
        });
        row.append(remove);
      }
      rowsBox.append(row);
    });
  }

  /**
   * What is left on the form itself: one line saying why it will not save.
   *
   * The totals moved into the confirmation dialog, and the panel that used to
   * hold them went with them — a bordered box sitting under the button, empty
   * most of the time, reads as something broken. What remains is a sentence in
   * the status line, because an expense that cannot be saved should say so
   * while it is being typed rather than after the button is pressed.
   */
  function renderPreview(p: Preview): void {
    feedback.textContent = p.blockers.concat(p.warnings).join(" · ");
    feedback.className = p.blockers.length > 0 ? "hint problem" : "hint";
    save.disabled = saving || p.blockers.length > 0;
  }

  // --- behaviour ------------------------------------------------------------

  async function refreshPreview(): Promise<void> {
    if (draft().rows.length === 0) {
      feedback.textContent = "";
      feedback.className = "hint";
      save.disabled = true;
      return;
    }
    try {
      renderPreview(await previewSubtotals(draft()));
    } catch (error) {
      feedback.textContent = String(error);
      feedback.className = "hint problem";
      save.disabled = true;
    }
  }

  const schedulePreview = debounce(150, () => void refreshPreview());

  async function loadContext(name: string | null): Promise<void> {
    // Leaving the store field commits it, whether or not a letter changed, so
    // this runs on every tab-out and every click into another field. Reloading
    // for a name we already hold would be an IPC round trip and a repaint for
    // no news.
    if (name === loadedFor) return;
    const ctx = await entryContext(name);
    loadedFor = name;
    locationName = ctx.locationName;
    categoryRates = new Map(ctx.categoryRates.map((r) => [r.categoryId, r.ratePpm]));
    favourites = ctx.recentMerchants;

    // The Location field says where this is being taxed, so the store no
    // longer needs a line under it repeating the same place name.
    //
    // Filled in only when the user has not chosen one themselves: naming a
    // store must not silently move an expense already placed somewhere.
    if (place.value() === "" || place.value() === lastDefaultPlace) {
      place.input.value = locationName ?? "";
    }
    lastDefaultPlace = locationName ?? "";

    // The store's remembered category is deliberately *not* filled in. It is
    // still remembered, and `pacc add` still applies it, but on this screen a
    // category that appears by itself is one the user did not choose and may
    // not notice — and the receipt in their hand is the authority on what they
    // bought. Naming Macey's should not decide that this was groceries.
    //
    // It also made "Save expense" look broken: the form cleared its rows and
    // the reloaded context put the category straight back, so the field never
    // appeared to empty.

    // The rates may have moved — a store in another town charges differently —
    // so the labels are repainted. In place, rather than by re-rendering the
    // rows: rebuilding them would destroy the input the user is clicking.
    for (const repaint of rateLabels) repaint();
    renderChips();
  }

  function renderChips(): void {
    clear(chips);
    for (const name of favourites) {
      const chip = el("button", { type: "button", class: "chip" }, [name]);
      chip.addEventListener("click", () => {
        merchant.input.value = name;
        void loadContext(name).then(() => schedulePreview());
      });
      chips.append(chip);
    }
    dropChipsThatDoNotFit();
  }

  /**
   * Keep the chips to one row the width of the Store field, dropping any that
   * would not fit whole.
   *
   * Clipping with `overflow: hidden` would leave a sliced chip at the edge —
   * half a shop name, still clickable, reading as a rendering fault. A chip
   * that cannot be shown completely is not shown at all.
   *
   * Measured rather than estimated, because a chip's width is its name in the
   * user's font at the user's zoom, and nothing in CSS can count how many of
   * those fit.
   */
  function dropChipsThatDoNotFit(): void {
    const limit = chips.clientWidth;
    // Zero while the tab is hidden or before first layout. Trimming against a
    // width of zero would drop every chip and never put them back.
    if (limit === 0) return;
    for (const chip of [...chips.children]) {
      if (!(chip instanceof HTMLElement)) continue;
      if (chip.offsetLeft + chip.offsetWidth > limit) chip.remove();
    }
  }

  addRow.addEventListener("click", () => {
    rows.push({ categoryId: null, subtotalText: "" });
    renderRows();
    schedulePreview();
  });

  save.addEventListener("click", () => {
    if (saving) return;
    saving = true;
    save.disabled = true;
    clear(feedback);

    // Nothing is written until the user has seen the numbers and said yes. The
    // dialog re-previews through the core on every correction, so what they
    // approve is exactly what gets saved — the same guarantee the form has,
    // carried through the editing step.
    void previewSubtotals(draft())
      .then((preview) =>
        reviewDialog({
          lines: linesFrom(preview),
          preview,
          recompute: (edits) => previewSubtotals(draft(withNotes(edits))),
        }),
      )
      .then(async (approved) => {
        if (approved === null) return;
        const id = await saveSubtotals(draft(withNotes(approved)));
        // A blank form, not a re-run of the last one. Leaving the store and
        // the amounts on screen invites a second click that files the same
        // expense twice, and every field that survives is one the next expense
        // has to be checked against.
        feedback.textContent = `Saved expense #${id}.`;
        merchant.input.value = "";
        place.input.value = "";
        note.value = "";
        date.setValue(today());
        rows = [{ categoryId: null, subtotalText: "" }];
        renderRows();
        // Forced past the "same merchant, nothing to do" guard, because one
        // thing *has* changed: this store was just shopped at, so the chips of
        // most-shopped stores are out of date by exactly this receipt.
        loadedFor = undefined;
        await loadContext(null);
        onSaved();
      })
      .catch((error: unknown) => {
        feedback.textContent = String(error);
      })
      .finally(() => {
        saving = false;
        save.disabled = false;
      });
  });

  /** The dialog's edits, as draft rows. It has no opinion about notes. */
  function withNotes(edits: ReviewResult[]): SubtotalRow[] {
    return edits.map((e) => ({
      categoryId: e.categoryId,
      subtotal: e.subtotal,
      tax: e.tax,
      note: null,
    }));
  }

  root.append(
    el("h1", {}, ["Add an expense"]),
    el("div", { class: "form" }, [
      el("label", { for: "f-date" }, ["Date"]),
      date.root,
      el("label", { for: "f-merchant" }, ["Store"]),
      el("div", {}, [merchant.root, chips]),
      el("label", {}, ["Categories"]),
      el("div", {}, [rowsBox, addRow]),
      el("label", { for: "f-place" }, ["Location"]),
      place.root,
      el("label", { for: "f-note" }, ["Note"]),
      note,
    ]),
    el("div", { class: "actions" }, [save, feedback]),
  );

  void (async () => {
    const [cats, stores, places, areas] = await Promise.all([
      listCategories(),
      listMerchants(),
      listLocations(),
      listAreas(),
    ]);
    categories = cats;
    merchant.setOptions(stores.map((m) => m.name));
    // Places already shopped first, then every other jurisdiction the state
    // publishes. A name from the second list becomes a real location on save,
    // with the published rate and a link that follows the quarterly refresh.
    const known = new Set(places.map((p) => p.name));
    place.setOptions([
      ...places.map((p) => p.name),
      ...areas.map((a) => a.name).filter((name) => !known.has(name)),
    ]);
    renderRows();
    await loadContext(null);
  })();

  return root;
}

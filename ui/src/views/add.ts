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
 * # Why the preview panel is the whole design
 *
 * Every change sends the rows to `preview_subtotals`, which reconciles the
 * amounts and apportions the tax in Rust and returns exactly what `save` would
 * write. The panel renders that answer verbatim, so the numbers on screen are
 * not the UI's opinion of the receipt — they *are* the receipt.
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
  listMerchants,
  previewSubtotals,
  saveSubtotals,
} from "../api.ts";
import { combobox } from "../combobox.ts";
import { dateField } from "../datefield.ts";
import { clear, debounce, el, today } from "../dom.ts";
import { formatMoney, formatRatePpm, parseCents } from "../money.ts";

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
  /** The merchant whose context is loaded, so leaving the field is free. */
  let loadedFor: string | null | undefined;
  let favourites: string[] = [];
  let saving = false;

  const merchant = combobox({
    id: "f-merchant",
    placeholder: "Costco",
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

  const note = el("input", { type: "text", id: "f-note", placeholder: "optional" });
  const chips = el("div", { class: "chips" });
  const context = el("p", { class: "hint" });
  const rowsBox = el("div", { class: "splits" });
  const addRow = el("button", { type: "button", class: "link" }, ["+ Add category"]);
  const panel = el("div", { class: "preview" });
  const save = el("button", { type: "button", class: "primary" }, ["Save receipt"]);
  const feedback = el("p", { class: "hint", role: "status" });

  // --- draft ----------------------------------------------------------------

  function draft(): SubtotalDraft {
    const entered: SubtotalRow[] = [];
    for (const row of rows) {
      const cents = parseCents(row.subtotalText);
      // A row still being typed is left out rather than sent as zero: a $0.00
      // line would claim the user meant to file nothing under that category.
      if (cents !== null) {
        entered.push({ categoryId: row.categoryId, subtotal: cents, note: null });
      }
    }
    const noteText = note.value.trim();
    return {
      id: null,
      txnDate: date.value(),
      merchant: merchant.value() || null,
      location: locationName,
      rows: entered,
      notes: noteText === "" ? null : noteText,
      needsReview: false,
      rememberDefaults: true,
    };
  }

  // --- rendering ------------------------------------------------------------

  function renderRows(): void {
    clear(rowsBox);
    rateLabels = [];
    rows.forEach((state, index) => {
      // The same filtering combobox as the store field: two dozen categories
      // is more than a dropdown is comfortable to scan, and typing three
      // letters beats scrolling on every receipt.
      const picker = combobox({
        placeholder: "Groceries",
        onCommit: (name) => {
          const match = categories.find((c) => c.name.toLowerCase() === name.toLowerCase());
          state.categoryId = match?.id ?? null;
          showRate();
          schedulePreview();
        },
      });
      picker.setOptions(categories.map((c) => c.name));
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
        rate.textContent = ppm === undefined ? "" : `${formatRatePpm(ppm)} sales tax`;
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
   * The rate a summary line was taxed at, as `"(3%)"` — empty when the
   * category is unknown.
   *
   * Read from the same core-supplied table the pickers above are labelled
   * with, so the summary and the rows can never print different figures for
   * one category.
   */
  function rowRate(categoryId: number | null): string {
    const ppm = categoryId === null ? undefined : categoryRates.get(categoryId);
    return ppm === undefined ? "" : `(${formatRatePpm(ppm)})`;
  }

  function renderPreview(p: Preview): void {
    clear(panel);

    // Subtotal, then one tax line per category, then the total. A single "Tax"
    // line cannot be honest here: the rate depends on the category, so on a
    // mixed receipt one figure would be labelled with a rate true of only part
    // of it. Naming each category's tax says where every cent came from, which
    // is the same question the rows above answer.
    const lines = [el("dt", {}, ["Subtotal"]), el("dd", {}, [formatMoney(p.subtotal)])];
    for (const row of p.rows) {
      lines.push(
        el("dt", {}, [`${row.categoryName ?? "— no category —"} tax ${rowRate(row.categoryId)}`]),
        el("dd", {}, [formatMoney(row.tax)]),
      );
    }
    lines.push(
      el("dt", { class: "grand" }, ["Total"]),
      el("dd", { class: "grand" }, [formatMoney(p.total)]),
    );
    panel.append(el("dl", { class: "totals" }, lines));

    for (const b of p.blockers) panel.append(el("p", { class: "blocker" }, [b]));
    for (const w of p.warnings) panel.append(el("p", { class: "warning" }, [w]));

    save.disabled = saving || p.blockers.length > 0;
  }

  // --- behaviour ------------------------------------------------------------

  async function refreshPreview(): Promise<void> {
    if (draft().rows.length === 0) {
      clear(panel);
      panel.append(el("p", { class: "hint" }, ["Enter what each category came to."]));
      save.disabled = true;
      return;
    }
    try {
      renderPreview(await previewSubtotals(draft()));
    } catch (error) {
      clear(panel);
      panel.append(el("p", { class: "blocker" }, [String(error)]));
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

    clear(context);
    context.append(locationName ?? "no location");

    // The store's remembered category is deliberately *not* filled in. It is
    // still remembered, and `pacc add` still applies it, but on this screen a
    // category that appears by itself is one the user did not choose and may
    // not notice — and the receipt in their hand is the authority on what they
    // bought. Naming Macey's should not decide that this was groceries.
    //
    // It also made "Save receipt" look broken: the form cleared its rows and
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
    void saveSubtotals(draft())
      .then((id) => {
        // Reset to a fresh receipt rather than leaving the saved one on
        // screen, where a second click would file it twice.
        feedback.textContent = `Saved receipt #${id}.`;
        note.value = "";
        rows = [{ categoryId: null, subtotalText: "" }];
        renderRows();
        clear(panel);
        void loadContext(merchant.value() || null);
        onSaved();
      })
      .catch((error: unknown) => {
        feedback.textContent = String(error);
      })
      .finally(() => {
        saving = false;
      });
  });

  root.append(
    el("h1", {}, ["Add a receipt"]),
    el("div", { class: "form" }, [
      el("label", { for: "f-date" }, ["Date"]),
      date.root,
      el("label", { for: "f-merchant" }, ["Store"]),
      el("div", {}, [merchant.root, chips, context]),
      el("label", {}, ["Categories"]),
      el("div", {}, [rowsBox, addRow]),
      el("label", { for: "f-note" }, ["Note"]),
      note,
    ]),
    el("div", { class: "actions" }, [save, feedback]),
    panel,
  );

  void (async () => {
    const [cats, stores] = await Promise.all([listCategories(), listMerchants()]);
    categories = cats;
    merchant.setOptions(stores.map((m) => m.name));
    renderRows();
    await loadContext(null);
  })();

  return root;
}

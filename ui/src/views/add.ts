/**
 * The receipt entry form.
 *
 * # Why the preview panel is the whole design
 *
 * Every field change sends the draft to `preview_transaction`, which reconciles
 * the amounts and plans the splits in Rust and sends back exactly what `save`
 * would write. The panel renders that answer verbatim. So the numbers on
 * screen are not the UI's opinion of the receipt — they *are* the receipt, and
 * there is no way for the display and the stored row to disagree.
 *
 * That is also why this file contains no arithmetic. It never subtracts tax
 * from a total or divides a split; it asks and it renders.
 *
 * # The flow, and why it is this short
 *
 * Date, store, amount, category — and the last two usually fill themselves in.
 * Naming a store recalls what was learned last time you shopped there: its
 * category, its location, and therefore its tax rate. The original design had
 * the user declare up front how many categories a receipt had; this asks for
 * one and offers "+ Add split", so the common single-category receipt costs
 * nothing and a split one is still possible.
 */

import {
  type Category,
  type Preview,
  type SplitDraft,
  type TransactionDraft,
  entryContext,
  listCategories,
  previewTransaction,
  saveTransaction,
} from "../api.ts";
import { dateField } from "../datefield.ts";
import { clear, debounce, el, today } from "../dom.ts";
import { formatMoney, formatRatePpm, parseCents } from "../money.ts";
import { parseSplitAmount } from "../splitspec.ts";

/** One split row's state. `amountText` empty means "the rest". */
interface SplitState {
  categoryId: number | null;
  amountText: string;
}

export function addView(onSaved: () => void): HTMLElement {
  const root = el("section", { class: "view" });

  let categories: Category[] = [];
  let splits: SplitState[] = [{ categoryId: null, amountText: "" }];
  let locationName: string | null = null;
  let taxRatePpm = 0;
  let recent: string[] = [];
  let saving = false;

  // --- fields ---------------------------------------------------------------
  const merchant = el("input", { type: "text", id: "f-merchant", placeholder: "Costco" });
  // Segmented MM/DD/YYYY rather than <input type="date">, whose per-segment
  // typing and Backspace behaviour belong to the webview engine. Enter hands
  // off to the store field, which is the next thing you fill in.
  const date = dateField({
    id: "f-date",
    onChange: () => schedulePreview(),
    onEnter: () => merchant.focus(),
  });
  date.setValue(today());
  const total = el("input", {
    type: "text",
    id: "f-total",
    inputmode: "decimal",
    placeholder: "0.00",
  });
  const tax = el("input", {
    type: "text",
    id: "f-tax",
    inputmode: "decimal",
    placeholder: "off the receipt",
  });
  const note = el("input", { type: "text", id: "f-note", placeholder: "optional" });

  const chips = el("div", { class: "chips" });
  const context = el("p", { class: "hint" });
  const splitRows = el("div", { class: "splits" });
  const addSplit = el("button", { type: "button", class: "link" }, ["+ Add split"]);
  const panel = el("div", { class: "preview" });
  const save = el("button", { type: "button", class: "primary" }, ["Save receipt"]);
  const feedback = el("p", { class: "hint", role: "status" });

  // --- draft assembly -------------------------------------------------------

  function draft(): TransactionDraft {
    const rows: SplitDraft[] = [];
    splits.forEach((state, i) => {
      const amount = parseSplitAmount(state.amountText, i === splits.length - 1);
      if (amount !== null) {
        rows.push({ categoryId: state.categoryId, amount, note: null });
      }
    });
    const noteText = note.value.trim();
    return {
      id: null,
      txnDate: date.value(),
      merchant: merchant.value.trim() || null,
      // The name is passed through only when the core already knows it; a
      // typo'd location is an error in the core rather than a silent 0% rate,
      // and there is no field here for inventing one.
      location: locationName,
      subtotal: null,
      tax: parseCents(tax.value),
      total: parseCents(total.value),
      splits: rows,
      detailLevel: null,
      notes: noteText === "" ? null : noteText,
      needsReview: false,
      rememberDefaults: true,
    };
  }

  // --- rendering ------------------------------------------------------------

  function renderSplits(): void {
    clear(splitRows);
    splits.forEach((state, index) => {
      const picker = el("select", { class: "category" });
      picker.append(el("option", { value: "" }, ["Choose a category…"]));
      for (const c of categories) {
        const option = el("option", { value: String(c.id) }, [`${c.groupName} / ${c.name}`]);
        if (state.categoryId === c.id) option.selected = true;
        picker.append(option);
      }
      picker.addEventListener("change", () => {
        const value = picker.value;
        state.categoryId = value === "" ? null : Number(value);
        schedulePreview();
      });

      const isLast = index === splits.length - 1;
      const amount = el("input", {
        type: "text",
        class: "amount-in",
        inputmode: "decimal",
        value: state.amountText,
        placeholder: isLast ? "the rest" : "0.00 or 25%",
      });
      amount.addEventListener("input", () => {
        state.amountText = amount.value;
        schedulePreview();
      });

      const row = el("div", { class: "split-row" }, [picker, amount]);
      if (splits.length > 1) {
        const remove = el("button", { type: "button", class: "link remove" }, ["×"]);
        remove.addEventListener("click", () => {
          splits.splice(index, 1);
          renderSplits();
          schedulePreview();
        });
        row.append(remove);
      }
      splitRows.append(row);
    });
  }

  function renderPreview(p: Preview): void {
    clear(panel);

    const summary = el("dl", { class: "totals" }, [
      el("dt", {}, ["Subtotal"]),
      el("dd", {}, [formatMoney(p.subtotal)]),
      el("dt", {}, ["Tax"]),
      el("dd", {}, [
        formatMoney(p.tax),
        ...(p.taxIsEstimated
          ? [el("span", { class: "estimated", title: "estimated, not read off the receipt" }, ["≈"])]
          : []),
      ]),
      el("dt", { class: "grand" }, ["Total"]),
      el("dd", { class: "grand" }, [formatMoney(p.total)]),
    ]);
    panel.append(summary);

    if (p.rows.length > 0) {
      const table = el("table", { class: "split-preview" });
      const body = el("tbody");
      for (const row of p.rows) {
        body.append(
          el("tr", {}, [
            el("td", {}, [row.categoryName ?? "— no category —"]),
            el("td", { class: "amount" }, [formatMoney(row.amount)]),
          ]),
        );
      }
      table.append(body);
      panel.append(table);
    }

    if (p.unassigned !== 0) {
      panel.append(
        el("p", { class: "blocker" }, [`${formatMoney(p.unassigned)} still unassigned`]),
      );
    }
    for (const b of p.blockers) panel.append(el("p", { class: "blocker" }, [b]));
    for (const w of p.warnings) panel.append(el("p", { class: "warning" }, [w]));

    save.disabled = saving || p.blockers.length > 0;
  }

  // --- behaviour ------------------------------------------------------------

  async function refreshPreview(): Promise<void> {
    if (parseCents(total.value) === null) {
      clear(panel);
      panel.append(el("p", { class: "hint" }, ["Enter a total to see the breakdown."]));
      save.disabled = true;
      return;
    }
    try {
      renderPreview(await previewTransaction(draft()));
    } catch (error) {
      clear(panel);
      panel.append(el("p", { class: "blocker" }, [String(error)]));
      save.disabled = true;
    }
  }

  const schedulePreview = debounce(150, () => void refreshPreview());

  async function loadContext(name: string | null): Promise<void> {
    const ctx = await entryContext(name);
    locationName = ctx.locationName;
    taxRatePpm = ctx.taxRatePpm;
    recent = ctx.recentMerchants;

    clear(context);
    const where = locationName ?? "no location";
    context.append(`${where} · ${formatRatePpm(taxRatePpm)} tax`);
    if (taxRatePpm === 0) {
      // Actionable, because the fix is one command away and otherwise every
      // receipt silently records no tax.
      context.append(
        el("span", { class: "warning-inline" }, [
          "  no rate set — `pacc location --name Home --rate 7.75`",
        ]),
      );
    }

    // Only prefill an untouched first row, so recalling a store's usual
    // category never overwrites a choice already made.
    if (
      ctx.defaultCategoryId !== null &&
      splits.length === 1 &&
      splits[0] !== undefined &&
      splits[0].categoryId === null
    ) {
      splits[0].categoryId = ctx.defaultCategoryId;
      renderSplits();
    }
    renderChips();
  }

  function renderChips(): void {
    clear(chips);
    for (const name of recent.slice(0, 6)) {
      const chip = el("button", { type: "button", class: "chip" }, [name]);
      chip.addEventListener("click", () => {
        merchant.value = name;
        void loadContext(name).then(() => schedulePreview());
      });
      chips.append(chip);
    }
  }

  merchant.addEventListener("change", () => {
    void loadContext(merchant.value.trim() || null).then(() => schedulePreview());
  });
  total.addEventListener("input", schedulePreview);
  tax.addEventListener("input", schedulePreview);


  addSplit.addEventListener("click", () => {
    splits.push({ categoryId: null, amountText: "" });
    renderSplits();
    schedulePreview();
  });

  save.addEventListener("click", () => {
    if (saving) return;
    saving = true;
    save.disabled = true;
    clear(feedback);
    void saveTransaction(draft())
      .then((id) => {
        // Reset to a fresh receipt rather than leaving the saved one on
        // screen, where a second click would file it twice.
        feedback.textContent = `Saved receipt #${id}.`;
        total.value = "";
        tax.value = "";
        note.value = "";
        splits = [{ categoryId: null, amountText: "" }];
        renderSplits();
        clear(panel);
        void loadContext(merchant.value.trim() || null);
        onSaved();
      })
      .catch((error: unknown) => {
        feedback.textContent = String(error);
      })
      .finally(() => {
        saving = false;
      });
  });

  // --- layout ---------------------------------------------------------------

  root.append(
    el("h1", {}, ["Add a receipt"]),
    el("div", { class: "form" }, [
      el("label", { for: "f-date" }, ["Date"]),
      date.root,
      el("label", { for: "f-merchant" }, ["Store"]),
      el("div", {}, [merchant, chips, context]),
      el("label", { for: "f-total" }, ["Total"]),
      total,
      el("label", { for: "f-tax" }, ["Tax"]),
      tax,
      el("label", {}, ["Categories"]),
      el("div", {}, [splitRows, addSplit]),
      el("label", { for: "f-note" }, ["Note"]),
      note,
    ]),
    el("div", { class: "actions" }, [save, feedback]),
    panel,
  );

  void (async () => {
    categories = await listCategories();
    renderSplits();
    await loadContext(null);
  })();

  return root;
}

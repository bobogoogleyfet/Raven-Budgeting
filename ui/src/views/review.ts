/**
 * The "is this right?" step, between pressing Save and writing anything.
 *
 * # Why the numbers are shown as a question
 *
 * The app works the tax out from a published rate, and a published rate is an
 * estimate: it is right for most receipts and wrong for the ones with a
 * bag-fee, a bottle deposit, a coupon applied after tax, or a jurisdiction the
 * quarterly file has not caught up with. The receipt in the user's hand is the
 * evidence, and this is where the two are put side by side while the paper is
 * still in front of them.
 *
 * So every category's amount and tax is editable here, and each edit is sent
 * back through the same `preview_subtotals` the form uses. The subtotal, the
 * total tax and the total are *not* editable: they are sums, and a sum you can
 * type is a sum that can disagree with its parts.
 */

import type { Preview } from "../api.ts";
import { el } from "../dom.ts";
import { type Cents, formatCents, formatMoney, parseCents } from "../money.ts";

/** One editable line, as the dialog holds it. */
export interface ReviewLine {
  categoryId: number | null;
  label: string;
  subtotalText: string;
  taxText: string;
  /**
   * Whether the tax figure is the user's rather than the app's.
   *
   * Only the app's are refreshed when something else changes: raising one
   * row's amount moves the tax on every row that shares its rate, and a
   * corrected figure must survive that.
   */
  taxEdited: boolean;
}

/** What the caller gets back: null if the user said no. */
export interface ReviewResult {
  categoryId: number | null;
  subtotal: number;
  /** `null` leaves the tax to the rate. */
  tax: number | null;
}

/** Build the starting lines from a preview of the untouched draft. */
export function linesFrom(preview: Preview): ReviewLine[] {
  return preview.rows.map((row) => ({
    categoryId: row.categoryId,
    label: row.categoryName ?? "— no category —",
    subtotalText: formatCents(row.subtotal),
    taxText: formatCents(row.tax),
    taxEdited: false,
  }));
}

/** The lines as the core wants them. */
export function editsFrom(lines: ReviewLine[]): ReviewResult[] {
  return lines.map((line) => ({
    categoryId: line.categoryId,
    subtotal: parseCents(line.subtotalText) ?? 0,
    // Only a figure the user actually touched is sent as an override. Sending
    // every row's tax back would make each save look like it had been read off
    // the paper, and `tax_is_estimated` would stop meaning anything.
    tax: line.taxEdited ? (parseCents(line.taxText) ?? 0) : null,
  }));
}

/**
 * Ask the user to confirm the receipt, letting them correct it first.
 *
 * `recompute` is the caller's route back to the core — the dialog never adds
 * two numbers itself, it asks what the numbers are.
 */
export function reviewDialog(config: {
  lines: ReviewLine[];
  preview: Preview;
  recompute: (edits: ReviewResult[]) => Promise<Preview>;
}): Promise<ReviewResult[] | null> {
  const lines = config.lines;

  return new Promise((resolve) => {
    const body = el("tbody");
    const subtotalCell = el("td", { class: "amount" });
    const taxCell = el("td", { class: "amount" });
    const totalCell = el("td", { class: "amount grand" });
    const problem = el("p", { class: "blocker" });

    const cancel = el("button", { type: "button", class: "secondary" }, ["Back"]);
    const go = el("button", { type: "button", class: "primary" }, ["Save expense"]);

    /** Tax inputs, so the computed ones can be refreshed after a change. */
    const taxInputs: HTMLInputElement[] = [];

    const show = (preview: Preview): void => {
      subtotalCell.textContent = formatMoney(preview.subtotal);
      taxCell.textContent = formatMoney(preview.tax);
      totalCell.textContent = formatMoney(preview.total);
      // A row the user has not corrected shows what the app now makes it:
      // changing one amount moves the tax on every row sharing its rate.
      preview.rows.forEach((row, index) => {
        const line = lines[index];
        const input = taxInputs[index];
        if (line === undefined || input === undefined || line.taxEdited) return;
        line.taxText = formatCents(row.tax);
        input.value = line.taxText;
      });
      problem.textContent = preview.blockers.join("; ");
      go.disabled = preview.blockers.length > 0;
    };

    let pending = 0;
    const refresh = (): void => {
      const mine = ++pending;
      void config
        .recompute(editsFrom(lines))
        .then((fresh) => {
          // A slower earlier request must not overwrite a newer answer.
          if (mine === pending) show(fresh);
        })
        .catch((error: unknown) => {
          problem.textContent = String(error);
          go.disabled = true;
        });
    };

    lines.forEach((line, index) => {
      const amount = el("input", {
        type: "text",
        class: "amount-in",
        inputmode: "decimal",
        value: line.subtotalText,
      });
      amount.addEventListener("input", () => {
        line.subtotalText = amount.value;
        refresh();
      });

      const tax = el("input", {
        type: "text",
        class: "amount-in",
        inputmode: "decimal",
        value: line.taxText,
      });
      tax.addEventListener("input", () => {
        line.taxText = tax.value;
        // From here this row's figure is the user's, and stays theirs.
        line.taxEdited = true;
        refresh();
      });
      taxInputs[index] = tax;

      body.append(
        el("tr", {}, [
          el("td", {}, [line.label]),
          el("td", { class: "amount" }, [amount]),
          el("td", { class: "amount" }, [tax]),
        ]),
      );
    });

    const table = el("table", { class: "review" }, [
      el("thead", {}, [
        el("tr", {}, [
          el("th", {}, ["Category"]),
          el("th", { class: "amount" }, ["Amount"]),
          el("th", { class: "amount" }, ["Tax"]),
        ]),
      ]),
      body,
      // Sums, and therefore not editable. The rows are what the receipt says;
      // these are what the rows come to.
      el("tfoot", {}, [
        el("tr", {}, [el("th", {}, ["Subtotal"]), subtotalCell, el("td", {}, [])]),
        el("tr", {}, [el("th", {}, ["Total tax"]), taxCell, el("td", {}, [])]),
        el("tr", { class: "grand" }, [el("th", {}, ["Total"]), totalCell, el("td", {}, [])]),
      ]),
    ]);

    const dialog = el("dialog", { class: "confirm review-dialog" }, [
      el("p", { class: "confirm-question" }, ["Does this match your receipt?"]),
      el("p", { class: "confirm-detail" }, [
        "Correct any amount or tax that differs from the paper — the receipt wins.",
      ]),
      table,
      problem,
      el("div", { class: "confirm-actions" }, [cancel, go]),
    ]);

    let answer: ReviewResult[] | null = null;
    dialog.addEventListener("close", () => {
      dialog.remove();
      resolve(answer);
    });

    cancel.addEventListener("click", () => dialog.close());
    go.addEventListener("click", () => {
      answer = editsFrom(lines);
      dialog.close();
    });

    document.body.append(dialog);
    if (typeof dialog.showModal === "function") {
      dialog.showModal();
    } else {
      dialog.setAttribute("open", "");
    }
    show(config.preview);
    go.focus();
  });
}

/** Re-exported so the caller can brand its own cents without a second import. */
export type { Cents };

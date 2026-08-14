/**
 * A modal confirmation, for the actions that cannot be taken back.
 *
 * # Why not `window.confirm`
 *
 * The native one is synchronous and blocks the webview's whole event loop, its
 * wording is fixed by the browser ("localhost says…"), and inside a Tauri
 * window it is at the mercy of the platform's webview rather than something
 * this app controls. A dialog the app builds says exactly what is about to
 * happen and names the thing it will happen to.
 *
 * Built on the native `<dialog>` element, so focus trapping, Escape, and the
 * backdrop come from the platform rather than from hand-written listeners that
 * would get them subtly wrong.
 */

import { el } from "./dom.ts";

export interface ConfirmRequest {
  /** The question, e.g. "Delete this receipt?" */
  question: string;
  /** What exactly is at stake — the row's own words, so the user can check. */
  detail?: string;
  /** The button that does the thing. Named for the action, never "OK". */
  confirmLabel: string;
}

/**
 * Ask, and resolve to what the user chose.
 *
 * Named `confirmDialog`, not `confirm`: a function called `confirm` shadows
 * `window.confirm`, so forgetting the import would silently fall back to the
 * native dialog this module exists to replace — and it would look like it
 * worked.
 *
 * Cancel is the default focus and the default outcome: Escape, the backdrop,
 * and closing the dialog any other way all resolve `false`. For a destructive
 * action the accident should be the harmless one.
 */
export function confirmDialog(request: ConfirmRequest): Promise<boolean> {
  return new Promise((resolve) => {
    const cancel = el("button", { type: "button", class: "secondary" }, ["Cancel"]);
    const go = el("button", { type: "button", class: "danger" }, [request.confirmLabel]);

    const children: (Node | string)[] = [el("p", { class: "confirm-question" }, [request.question])];
    if (request.detail !== undefined) {
      children.push(el("p", { class: "confirm-detail" }, [request.detail]));
    }
    children.push(el("div", { class: "confirm-actions" }, [cancel, go]));

    const dialog = el("dialog", { class: "confirm" }, children);

    // Resolves exactly once however the dialog goes away — including the
    // Escape key, which fires `close` without either button being touched.
    let answer = false;
    const finish = (): void => {
      dialog.remove();
      resolve(answer);
    };
    dialog.addEventListener("close", finish);

    cancel.addEventListener("click", () => dialog.close());
    go.addEventListener("click", () => {
      answer = true;
      dialog.close();
    });

    document.body.append(dialog);
    dialog.showModal();
    // Focus the safe choice, so a stray Return does not delete anything.
    cancel.focus();
  });
}

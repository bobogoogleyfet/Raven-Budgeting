/**
 * The handful of glyphs the interface draws rather than types.
 *
 * # Why not characters
 *
 * ▾, ▼, 🗑 and friends render as a caret, a glyph or a colour emoji depending
 * on the platform's font, at whatever weight and baseline that font decides.
 * Beside a 0.78rem header one is invisible and another is a black lozenge, and
 * two controls meant to look alike end up looking nothing alike.
 *
 * Drawn once here, they are the same shape everywhere — which is the point: a
 * dropdown's arrow and the sort arrow are the same idea and should not be two
 * different triangles. `currentColor` and `1em` sizing make them follow the
 * text they sit in, so hover, the active state and the dark theme all come for
 * free.
 */

/** A triangle, pointing down or up. */
export function caret(down: boolean): string {
  return (
    `<svg viewBox="0 0 10 10" width="8" height="8" aria-hidden="true" focusable="false">` +
    `<path d="${down ? "M1 3.5h8L5 8z" : "M1 6.5h8L5 2z"}" fill="currentColor"/></svg>`
  );
}

/** The funnel a spreadsheet puts in a filterable column, solid once it bites. */
export function funnel(filled: boolean): string {
  return (
    `<svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true" focusable="false">` +
    `<path d="M2 3h12l-4.6 5.4v4.2l-2.8 1.4V8.4z" ` +
    (filled
      ? `fill="currentColor"/>`
      : `fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/>`) +
    `</svg>`
  );
}

/**
 * A waste basket, sized in `em` so it matches whatever row it sits in.
 *
 * Stroked rather than filled: a solid black bin in a table of numbers pulls
 * the eye to the one control nobody should be drawn to.
 */
export function trash(): string {
  return (
    `<svg viewBox="0 0 16 16" width="1em" height="1em" aria-hidden="true" focusable="false">` +
    `<path d="M2.5 4h11M6.5 4V2.5h3V4M4 4l.7 9.2a1 1 0 0 0 1 .8h4.6a1 1 0 0 0 1-.8L12 4"` +
    ` fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"` +
    ` stroke-linejoin="round"/>` +
    `<path d="M6.6 6.5v5M9.4 6.5v5" fill="none" stroke="currentColor" stroke-width="1.2"` +
    ` stroke-linecap="round"/></svg>`
  );
}

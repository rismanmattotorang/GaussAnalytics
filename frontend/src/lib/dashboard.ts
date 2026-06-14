// Pure dashboard helpers — layout ordering, cross-filtering, reordering.

import type { Dashboard } from "../api/client";

export interface LayoutItem {
  card_id: string;
  w: number;
}

/** The display layout: the saved layout if present, else one half-width tile per card. */
export function orderedLayout(d: Dashboard): LayoutItem[] {
  if (d.layout && d.layout.length > 0) {
    return d.layout.map((l) => ({ card_id: l.card_id, w: l.w || 1 }));
  }
  return d.card_ids.map((id) => ({ card_id: id, w: 1 }));
}

/** The dashboard parameter (if any) a clicked column should cross-filter. */
export function matchingParam(
  parameters: Array<{ name: string }> | undefined,
  column: string,
): string | null {
  return parameters?.find((p) => p.name === column)?.name ?? null;
}

/** Move element `from` → `to`, returning a new array (used by drag reorder). */
export function move<T>(arr: T[], from: number, to: number): T[] {
  if (from === to || from < 0 || to < 0 || from >= arr.length || to >= arr.length) {
    return arr.slice();
  }
  const copy = arr.slice();
  const [x] = copy.splice(from, 1);
  copy.splice(to, 0, x);
  return copy;
}

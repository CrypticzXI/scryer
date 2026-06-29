import type { CSSProperties } from "react";

import type { WizardFacet } from "@/lib/hooks/use-external-import-setup";

/**
 * Facet identity colors for the import wizard's mapping board, mirroring the
 * design hand-off's `facetColor()`:
 *   Movies → themeable accent · Series → sky · Anime → violet.
 * Movies uses the runtime-tunable `--scry-accent*` vars (never hardcode the
 * brand hex); Series/Anime use literal tints from the design tokens.
 */
export interface FacetStyle {
  dot: string;
  bg: string;
  border: string;
  text: string;
}

const FACET_STYLES: Record<WizardFacet, FacetStyle> = {
  movie: {
    dot: "var(--scry-accent)",
    bg: "rgba(var(--scry-accent-rgb), 0.13)",
    border: "var(--scry-baccent)",
    text: "var(--scry-accent-text)",
  },
  series: {
    dot: "#38bdf8",
    bg: "rgba(56, 189, 248, 0.12)",
    border: "rgba(56, 189, 248, 0.3)",
    text: "#7dd3fc",
  },
  anime: {
    dot: "#9b5bff",
    bg: "rgba(155, 91, 255, 0.14)",
    border: "rgba(155, 91, 255, 0.32)",
    text: "#c4a3ff",
  },
};

export function facetStyle(facet: WizardFacet): FacetStyle {
  return FACET_STYLES[facet];
}

/** Inline style for a facet pill (dot + label chip). */
export function facetPillStyle(facet: WizardFacet): CSSProperties {
  const style = facetStyle(facet);
  return {
    background: style.bg,
    border: `1px solid ${style.border}`,
    color: style.text,
  };
}

/** i18n key for a facet's display label. */
export function facetLabelKey(facet: WizardFacet): string {
  switch (facet) {
    case "movie":
      return "setup.facetMovies";
    case "series":
      return "setup.facetSeries";
    case "anime":
      return "setup.facetAnime";
  }
}

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
    dot: "var(--scry-facet-movie)",
    bg: "var(--scry-facet-movie-bg)",
    border: "var(--scry-facet-movie-border)",
    text: "var(--scry-facet-movie-text)",
  },
  series: {
    dot: "var(--scry-facet-series)",
    bg: "var(--scry-facet-series-bg)",
    border: "var(--scry-facet-series-border)",
    text: "var(--scry-facet-series-text)",
  },
  anime: {
    dot: "var(--scry-facet-anime)",
    bg: "var(--scry-facet-anime-bg)",
    border: "var(--scry-facet-anime-border)",
    text: "var(--scry-facet-anime-text)",
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

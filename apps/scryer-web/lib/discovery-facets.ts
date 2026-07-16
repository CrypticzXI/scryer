import type { DiscoveryItem } from "@/lib/types";

export type CanonicalDiscoveryFacetKind = "genre" | "theme";

export function canonicalDiscoveryFacetLabel(
  value: string,
  kind: CanonicalDiscoveryFacetKind,
): string | null {
  const parts = value.trim().split(":");
  if (
    parts.length < 3 ||
    parts[0]?.toLowerCase() !== "canonical" ||
    parts[1]?.toLowerCase() !== kind
  ) {
    return null;
  }
  const slug = parts.slice(2).join(":").trim();
  if (!slug) {
    return null;
  }
  return slug
    .split(/[-_:\s]+/)
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1).toLowerCase())
    .join(" ");
}

export function canonicalDiscoveryLabelKey(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim();
}

export function canonicalDiscoveryFacetLabels(
  item: Pick<DiscoveryItem, "facetTerms">,
  kind: CanonicalDiscoveryFacetKind,
): string[] {
  const labels: string[] = [];
  const seen = new Set<string>();
  for (const term of item.facetTerms) {
    const label = canonicalDiscoveryFacetLabel(term, kind);
    const key = label ? canonicalDiscoveryLabelKey(label) : null;
    if (label && key && !seen.has(key)) {
      seen.add(key);
      labels.push(label);
    }
  }
  return labels;
}

export function canonicalDiscoveryFilterOptions(
  items: Array<Pick<DiscoveryItem, "facetTerms">>,
  kind: CanonicalDiscoveryFacetKind,
): string[] {
  const labels = new Set<string>();
  for (const item of items) {
    for (const label of canonicalDiscoveryFacetLabels(item, kind)) {
      labels.add(label);
    }
  }
  return [...labels].sort((left, right) => left.localeCompare(right));
}

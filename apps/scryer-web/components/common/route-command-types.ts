import type { LucideIcon } from "lucide-react";

export type RouteCommandItem = {
  id: string;
  label: string;
  description: string;
  icon?: LucideIcon;
  keywords?: string[];
  onSelect: () => void;
};

export type RouteCommandPaletteConfig = {
  title: string;
  description: string;
  placeholder: string;
  noResultsText: string;
  groupLabel: string;
  items: RouteCommandItem[];
};

function normalizeRouteCommandSearchText(value: string): string {
  return value.trim().toLowerCase();
}

function routeCommandSearchRank(
  item: RouteCommandItem,
  terms: string[],
  query: string,
): number | null {
  const label = normalizeRouteCommandSearchText(item.label);
  const description = normalizeRouteCommandSearchText(item.description);
  const keywords = (item.keywords ?? [])
    .map(normalizeRouteCommandSearchText)
    .filter(Boolean);
  const searchableValues = [label, description, ...keywords].filter(Boolean);
  const searchable = searchableValues.join(" ");

  if (!terms.every((term) => searchable.includes(term))) {
    return null;
  }

  if (label === query) return 0;
  if (label.startsWith(query)) return 1;
  if (keywords.some((keyword) => keyword === query)) return 2;
  if (keywords.some((keyword) => keyword.startsWith(query))) return 3;
  if (description === query) return 4;
  if (description.startsWith(query)) return 5;

  if (terms.every((term) => label.includes(term))) {
    return 10 + Math.min(...terms.map((term) => label.indexOf(term)));
  }

  if (terms.every((term) => description.includes(term))) {
    return 20 + Math.min(...terms.map((term) => description.indexOf(term)));
  }

  return 30 + Math.max(0, searchable.indexOf(terms[0] ?? ""));
}

export function filterRouteCommandItems(
  items: RouteCommandItem[],
  query: string,
): RouteCommandItem[] {
  const normalizedQuery = normalizeRouteCommandSearchText(query);
  const terms = normalizedQuery.split(/\s+/).filter(Boolean);

  if (terms.length === 0) {
    return items;
  }

  return items
    .map((item, index) => ({
      item,
      index,
      rank: routeCommandSearchRank(item, terms, normalizedQuery),
    }))
    .filter((match): match is { item: RouteCommandItem; index: number; rank: number } => match.rank !== null)
    .sort((a, b) => a.rank - b.rank || a.index - b.index)
    .map((match) => match.item);
}

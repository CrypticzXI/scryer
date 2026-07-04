import type { CanonicalMediaTag } from "@/lib/types/titles";

type TitleGenreSource = {
  canonicalTags?: CanonicalMediaTag[] | null;
  genres?: string[] | null;
};

export function titleGenreLabels(title: TitleGenreSource): string[] {
  const canonicalTags = title.canonicalTags ?? [];
  if (canonicalTags.length > 0) {
    return uniqueLabels(
      canonicalTags
        .filter((tag) => tag.category.trim().toLowerCase() === "genre")
        .map((tag) => tag.name),
    );
  }

  return uniqueLabels(title.genres ?? []);
}

function uniqueLabels(values: string[]): string[] {
  const labels: string[] = [];
  const seen = new Set<string>();
  for (const value of values) {
    const label = value.trim();
    const key = label.toLowerCase();
    if (!label || seen.has(key)) {
      continue;
    }
    seen.add(key);
    labels.push(label);
  }
  return labels;
}

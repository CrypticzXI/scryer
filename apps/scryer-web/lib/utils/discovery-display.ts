import type { DiscoveryItem } from "@/lib/types";

const SOURCE_ID_PATTERN = /^[a-z][a-z0-9_+-]*:[a-z0-9_+-]+:/i;
const NUMERIC_ID_PATTERN = /^\d+$/;

function hasNonLatinTitleCharacters(value: string) {
  for (const character of value) {
    if (character.codePointAt(0)! > 0x024f) {
      return true;
    }
  }
  return false;
}

function usefulAlternateTitle(value: string | null | undefined) {
  const title = value?.trim() ?? "";
  if (!title) {
    return null;
  }
  if (SOURCE_ID_PATTERN.test(title) || NUMERIC_ID_PATTERN.test(title)) {
    return null;
  }
  return title;
}

export function discoveryItemDisplayTitle(item: DiscoveryItem) {
  const displayTitle = item.displayTitle.trim();
  const alternateTitle = usefulAlternateTitle(item.originalTitle);
  if (
    alternateTitle &&
    (!displayTitle || hasNonLatinTitleCharacters(displayTitle))
  ) {
    return alternateTitle;
  }
  return displayTitle || alternateTitle || item.targetKey;
}

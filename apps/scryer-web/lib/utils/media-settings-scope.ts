import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import type { MediaSettings } from "@/lib/types/settings";

export function facetScopedMediaSettingsScopeId(
  mediaSettings: Pick<MediaSettings, "scope">,
): ViewCategoryId {
  return mediaSettings.scope;
}

export function updateFacetScopedStringRecord(
  previous: Record<ViewCategoryId, string>,
  scopeId: ViewCategoryId,
  nextValue: string,
): Record<ViewCategoryId, string> {
  if (previous[scopeId] === nextValue) {
    return previous;
  }
  return { ...previous, [scopeId]: nextValue };
}

export function updateFacetScopedStringArrayRecord(
  previous: Record<ViewCategoryId, string[]>,
  scopeId: ViewCategoryId,
  nextValues: string[],
): Record<ViewCategoryId, string[]> {
  const currentValues = previous[scopeId] ?? [];
  const same =
    currentValues.length === nextValues.length &&
    currentValues.every((value, index) => value === nextValues[index]);
  if (same) {
    return previous;
  }
  return { ...previous, [scopeId]: nextValues };
}

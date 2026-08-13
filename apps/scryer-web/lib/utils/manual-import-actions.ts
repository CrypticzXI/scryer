export function manualImportActions({
  displayState,
  facet,
  hasTitle,
}: {
  displayState: string;
  facet: string | null;
  hasTitle: boolean;
}) {
  const actionable =
    displayState === "IMPORT_BLOCKED" || displayState === "IMPORT_FAILED";
  const normalizedFacet = facet?.trim().toLowerCase() ?? "";

  return {
    interactive:
      hasTitle &&
      actionable &&
      (normalizedFacet === "series" || normalizedFacet === "anime"),
    direct: hasTitle && actionable && normalizedFacet === "movie",
  };
}

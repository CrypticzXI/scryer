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

  return {
    interactive:
      hasTitle && actionable && (facet === "series" || facet === "anime"),
    direct: hasTitle && actionable && facet === "movie",
  };
}

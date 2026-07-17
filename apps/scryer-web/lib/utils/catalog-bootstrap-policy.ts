export type CatalogSurfacePhase =
  | "resolving"
  | "content"
  | "empty"
  | "rootsMissing"
  | "rootsInvalid"
  | "error";

export type CatalogRootValidationState =
  | "notRun"
  | "valid"
  | "invalid"
  | "unavailable";

type CatalogSurfacePhaseInput = {
  canManageLibrarySettings: boolean;
  hasConfiguredRoots: boolean;
  loadedTitleCount: number | null;
  rootValidationState: CatalogRootValidationState;
};

export function resolveCatalogSurfacePhase({
  canManageLibrarySettings,
  hasConfiguredRoots,
  loadedTitleCount,
  rootValidationState,
}: CatalogSurfacePhaseInput): CatalogSurfacePhase {
  if (canManageLibrarySettings && !hasConfiguredRoots) {
    return "rootsMissing";
  }
  if (loadedTitleCount === null) {
    return "resolving";
  }
  if (loadedTitleCount > 0) {
    return "content";
  }
  return rootValidationState === "invalid" ? "rootsInvalid" : "empty";
}

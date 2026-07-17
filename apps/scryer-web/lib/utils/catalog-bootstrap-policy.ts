export type CatalogSurfacePhase =
  | "resolving"
  | "content"
  | "empty"
  | "rootsMissing"
  | "error";

type CatalogSurfacePhaseInput = {
  canManageLibrarySettings: boolean;
  hasConfiguredRoots: boolean;
  loadedTitleCount: number | null;
};

export function resolveCatalogSurfacePhase({
  canManageLibrarySettings,
  hasConfiguredRoots,
  loadedTitleCount,
}: CatalogSurfacePhaseInput): CatalogSurfacePhase {
  if (canManageLibrarySettings && !hasConfiguredRoots) {
    return "rootsMissing";
  }
  if (loadedTitleCount === null) {
    return "resolving";
  }
  return loadedTitleCount > 0 ? "content" : "empty";
}

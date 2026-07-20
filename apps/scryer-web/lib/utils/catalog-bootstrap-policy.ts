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

type CatalogLibraryRootState = {
  isBootstrapDefaultRootSet?: boolean;
  roots: readonly { path: string }[];
};

export function configuredCatalogLibraries<T extends CatalogLibraryRootState>(
  libraries: readonly T[],
  invalidPaths: readonly string[] = [],
): T[] {
  const invalidPathSet = new Set(invalidPaths);

  return libraries.filter((library) => {
    const rootPaths = library.roots
      .map((root) => root.path.trim())
      .filter((path) => path.length > 0);
    if (rootPaths.length === 0) {
      return false;
    }
    return !(
      library.isBootstrapDefaultRootSet === true &&
      rootPaths.every((path) => invalidPathSet.has(path))
    );
  });
}

export function catalogRootValidationState(result: {
  validPaths: readonly string[];
  invalidPaths: readonly string[];
  unavailable: boolean;
}): CatalogRootValidationState {
  if (result.validPaths.length > 0) {
    return "valid";
  }
  if (result.unavailable) {
    return "unavailable";
  }
  return result.invalidPaths.length > 0 ? "invalid" : "valid";
}

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

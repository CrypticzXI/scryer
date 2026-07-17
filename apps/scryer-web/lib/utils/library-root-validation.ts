type GraphQlErrorLike = {
  extensions?: Record<string, unknown> | null;
};

type CombinedErrorLike = {
  graphQLErrors?: readonly GraphQlErrorLike[];
};

export type LibraryRootValidationResult = {
  invalidPaths: string[];
  unavailable: boolean;
};

function isWindowsDriveRoot(path: string): boolean {
  return /^[a-z]:[\\/]$/iu.test(path);
}

function isUncShareRoot(path: string): boolean {
  if (!/^[\\/]{2}[^\\/]/u.test(path)) return false;
  return path.slice(2).split(/[\\/]+/u).filter(Boolean).length === 2;
}

export function trimLibraryRootPath(path: string): string {
  const trimmed = path.trim();
  if (!trimmed) return "";
  if (/^[\\/]+$/u.test(trimmed)) return trimmed[0] ?? "/";
  if (isWindowsDriveRoot(trimmed) || isUncShareRoot(trimmed)) return trimmed;
  return trimmed.replace(/[\\/]+$/u, "");
}

export function normalizeComparableLibraryRootPath(path: string): string {
  const storedPath = trimLibraryRootPath(path);
  const canonicalPath = isUncShareRoot(storedPath)
    ? storedPath.replace(/[\\/]+$/u, "")
    : storedPath;
  return canonicalPath.replaceAll("\\", "/").toLowerCase();
}

export function isExplicitPathValidationError(error: unknown): boolean {
  if (typeof error !== "object" || error === null) return false;
  const graphQLErrors = (error as CombinedErrorLike).graphQLErrors ?? [];
  return graphQLErrors.some(
    (graphQlError) => graphQlError.extensions?.code === "VALIDATION_ERROR",
  );
}

export async function validateLibraryRootPaths(
  paths: string[],
  validate: (path: string) => Promise<unknown | null | undefined>,
  concurrency = 4,
): Promise<LibraryRootValidationResult> {
  const invalidPaths: string[] = [];
  let unavailable = false;
  let nextIndex = 0;

  const worker = async () => {
    while (nextIndex < paths.length) {
      const index = nextIndex;
      nextIndex += 1;
      const path = paths[index];
      if (path === undefined) continue;

      try {
        const error = await validate(path);
        if (!error) continue;
        if (isExplicitPathValidationError(error)) {
          invalidPaths.push(path);
        } else {
          unavailable = true;
        }
      } catch {
        unavailable = true;
      }
    }
  };

  const workerCount = Math.min(Math.max(1, concurrency), paths.length);
  await Promise.all(Array.from({ length: workerCount }, () => worker()));
  return { invalidPaths, unavailable };
}

import {
  validateLibraryRootPaths,
  type LibraryRootValidationResult,
} from "./library-root-validation.ts";

export type SetupMediaPathField = "movies" | "series" | "anime";
export type InvalidSetupMediaPathFields = Partial<
  Record<SetupMediaPathField, boolean>
>;

export type SetupMediaPathsInput = {
  moviePath: string;
  seriesPath: string;
  animePath: string | null;
};

export type SetupMediaPathValidationState = {
  invalidPathFields: InvalidSetupMediaPathFields;
  unavailable: boolean;
};

type AdvisorySetupMediaPathSaveOptions = {
  input: SetupMediaPathsInput;
  validatePath: (path: string) => Promise<unknown | null | undefined>;
  savePaths: (input: SetupMediaPathsInput) => Promise<void>;
  onValidation: (state: SetupMediaPathValidationState) => void;
  onSaved: (state: SetupMediaPathValidationState) => void;
};

function validationState(
  candidates: Array<{ field: SetupMediaPathField; path: string }>,
  result: LibraryRootValidationResult,
): SetupMediaPathValidationState {
  const invalidPaths = new Set(result.invalidPaths);
  const invalidPathFields: InvalidSetupMediaPathFields = {};
  candidates.forEach(({ field, path }) => {
    if (invalidPaths.has(path)) {
      invalidPathFields[field] = true;
    }
  });
  return { invalidPathFields, unavailable: result.unavailable };
}

export async function runAdvisorySetupMediaPathSave({
  input,
  validatePath,
  savePaths,
  onValidation,
  onSaved,
}: AdvisorySetupMediaPathSaveOptions): Promise<void> {
  const allCandidates: Array<{ field: SetupMediaPathField; path: string }> = [
    { field: "movies", path: input.moviePath },
    { field: "series", path: input.seriesPath },
    { field: "anime", path: input.animePath ?? "" },
  ];
  const candidates = allCandidates.filter(({ path }) => path.length > 0);

  const result = await validateLibraryRootPaths(
    candidates.map(({ path }) => path),
    validatePath,
  );
  const state = validationState(candidates, result);
  onValidation(state);

  await savePaths(input);
  onSaved(state);
}

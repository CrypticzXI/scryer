export function editDialogTargets<T>(
  directTarget: T | null,
  bulkTargets: readonly T[],
): T[] {
  return directTarget === null ? [...bulkTargets] : [directTarget];
}

export const UNCHANGED_TITLE_EDIT_VALUE = "__unchanged__";
export const INHERIT_TITLE_EDIT_VALUE = "__inherit__";
export const ENABLED_TITLE_EDIT_VALUE = "enabled";
export const DISABLED_TITLE_EDIT_VALUE = "disabled";

export type TitleEditDraft = {
  qualityProfileId: string;
  rootFolderId: string;
  monitorType: string;
  useSeasonFolders: string;
  monitorSpecials: string;
  interSeasonMovies: string;
  fillerPolicy: string;
  recapPolicy: string;
};

type TitleEditSource = {
  qualityProfileId?: string | null;
  rootFolderId?: string | null;
  monitorType?: string | null;
  useSeasonFolders?: boolean | null;
  monitorSpecials?: boolean | null;
  interSeasonMovies?: boolean | null;
  fillerPolicy?: string | null;
  recapPolicy?: string | null;
};

function booleanDraftValue(value: boolean | null | undefined): string {
  if (value === true) {
    return ENABLED_TITLE_EDIT_VALUE;
  }
  if (value === false) {
    return DISABLED_TITLE_EDIT_VALUE;
  }
  return UNCHANGED_TITLE_EDIT_VALUE;
}

function inheritedDraftValue(value: string | null | undefined): string {
  return value?.trim() ? value : INHERIT_TITLE_EDIT_VALUE;
}

export function initialTitleEditDraft(
  directTarget: TitleEditSource | null,
): TitleEditDraft {
  if (directTarget === null) {
    return {
      qualityProfileId: UNCHANGED_TITLE_EDIT_VALUE,
      rootFolderId: UNCHANGED_TITLE_EDIT_VALUE,
      monitorType: UNCHANGED_TITLE_EDIT_VALUE,
      useSeasonFolders: UNCHANGED_TITLE_EDIT_VALUE,
      monitorSpecials: UNCHANGED_TITLE_EDIT_VALUE,
      interSeasonMovies: UNCHANGED_TITLE_EDIT_VALUE,
      fillerPolicy: UNCHANGED_TITLE_EDIT_VALUE,
      recapPolicy: UNCHANGED_TITLE_EDIT_VALUE,
    };
  }

  return {
    qualityProfileId: inheritedDraftValue(directTarget.qualityProfileId),
    rootFolderId: directTarget.rootFolderId ?? UNCHANGED_TITLE_EDIT_VALUE,
    monitorType: directTarget.monitorType ?? UNCHANGED_TITLE_EDIT_VALUE,
    useSeasonFolders: booleanDraftValue(directTarget.useSeasonFolders),
    monitorSpecials: booleanDraftValue(directTarget.monitorSpecials),
    interSeasonMovies: booleanDraftValue(directTarget.interSeasonMovies),
    fillerPolicy: inheritedDraftValue(directTarget.fillerPolicy),
    recapPolicy: inheritedDraftValue(directTarget.recapPolicy),
  };
}

export function hasTitleEditChanges(
  draft: TitleEditDraft,
  initialDraft: TitleEditDraft,
): boolean {
  return (Object.keys(draft) as Array<keyof TitleEditDraft>).some(
    (key) =>
      draft[key] !== initialDraft[key] &&
      draft[key] !== UNCHANGED_TITLE_EDIT_VALUE,
  );
}

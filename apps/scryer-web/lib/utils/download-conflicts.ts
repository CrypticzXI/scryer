export type DownloadConflictLike = {
  replaceable?: boolean | null;
  sourceTitle?: string | null;
  titleName?: string | null;
  state?: string | null;
};

type ConflictResultLike = {
  status?: string | null;
  conflict?: DownloadConflictLike | null;
};

export async function retryWithReplaceOnConflict<
  TInput extends object,
  TPayload extends ConflictResultLike | null | undefined,
>(
  input: TInput,
  submit: (input: TInput) => Promise<TPayload>,
  conflictMessage: string,
  confirmReplace: (
    conflict: DownloadConflictLike,
    conflictMessage: string,
  ) => Promise<boolean>,
): Promise<TPayload> {
  const first = await submit(input);
  const conflict = first?.conflict;
  if (first?.status !== "conflict" && !conflict) {
    return first;
  }

  if (!conflict?.replaceable) {
    throw new Error(conflictMessage);
  }

  const confirmed = await confirmReplace(conflict, conflictMessage);
  if (!confirmed) {
    throw new Error(conflictMessage);
  }

  return submit({ ...input, replaceInProgress: true } as TInput);
}

export function assertNoReplaceConflict(
  payload: ConflictResultLike | null | undefined,
  conflictMessage: string,
) {
  if (payload?.status === "conflict" || payload?.conflict) {
    throw new Error(conflictMessage);
  }
}

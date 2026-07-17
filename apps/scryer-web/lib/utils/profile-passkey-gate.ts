export function shouldLoadProfilePasskeys({
  authLoading,
  effectiveFormLoginEnabled,
  passkeyEnabled,
  userId,
  accountKind,
}: {
  authLoading: boolean;
  effectiveFormLoginEnabled: boolean | undefined;
  passkeyEnabled: boolean;
  userId: string | null | undefined;
  accountKind: string | null;
}): boolean {
  return (
    !authLoading
    && effectiveFormLoginEnabled === true
    && passkeyEnabled
    && Boolean(userId)
    && accountKind === "LOCAL"
  );
}

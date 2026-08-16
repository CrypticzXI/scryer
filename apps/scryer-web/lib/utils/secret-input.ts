export function nonEmptySecret(value: string): string | undefined {
  return value.length > 0 ? value : undefined;
}

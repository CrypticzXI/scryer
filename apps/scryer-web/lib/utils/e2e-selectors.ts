export function selectorToken(value: string | number): string {
  const normalized = String(value)
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/(^-|-$)+/g, "");

  return normalized || "item";
}

export function selectorId(
  ...parts: Array<string | number | false | null | undefined>
): string {
  return parts
    .filter(
      (part): part is string | number =>
        part !== false &&
        part !== null &&
        part !== undefined &&
        String(part).trim().length > 0,
    )
    .map((part) => selectorToken(part))
    .join("-");
}

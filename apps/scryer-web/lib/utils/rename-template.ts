export type RenameTemplateValidationIssue =
  | { kind: "empty" }
  | { kind: "unmatchedOpen" }
  | { kind: "unmatchedClose" }
  | { kind: "unknownToken"; token: string }
  | { kind: "invalidPadding"; padding: string }
  | { kind: "invalidFilter"; filter: string };

export type FolderTemplateValidationIssue =
  | RenameTemplateValidationIssue
  | { kind: "illegalCharacter"; character: string }
  | { kind: "missingRequiredToken"; token: string };

export type RenameTemplateSegment = {
  text: string;
  isToken: boolean;
};

export type RenameTokenFilter =
  | {
      kind: "space";
      replacement: string;
    }
  | {
      kind: "truncate";
      limit: number;
    };

export type ParsedRenameTokenSpec = {
  tokenName: string;
  lookupName: string;
  padWidth: number;
  filters: RenameTokenFilter[];
};

export const MAX_RENAME_TEMPLATE_PADDING_WIDTH = 240;

export type RenameTokenParseResult =
  | { ok: true; spec: ParsedRenameTokenSpec }
  | { ok: false; kind: "emptyToken" | "invalidFilter" | "invalidPadding"; value: string };

const SPACE_FILTER_PREFIX = "space:";
const TRUNCATE_FILTER_PREFIX = "truncate:";
const VALID_SPACE_REPLACEMENTS = new Set(["_", ".", "-", ""]);
const ILLEGAL_FOLDER_TEMPLATE_LITERAL_CHARS = new Set([
  "<", ">", ":", "\"", "/", "\\", "|", "?", "*",
]);

function isIllegalFolderTemplateLiteral(character: string): boolean {
  const codePoint = character.charCodeAt(0);
  return ILLEGAL_FOLDER_TEMPLATE_LITERAL_CHARS.has(character)
    || codePoint <= 0x1f
    || (codePoint >= 0x7f && codePoint <= 0x9f);
}

export function validateFolderTemplateSyntax(
  template: string,
  validTokens: ReadonlySet<string>,
  requiredToken?: string,
): FolderTemplateValidationIssue | null {
  const trimmed = template.trim();
  if (!trimmed) {
    return { kind: "empty" };
  }

  let i = 0;
  let sawRequiredToken = requiredToken === undefined;
  while (i < trimmed.length) {
    if (trimmed[i] === "{") {
      const closeIndex = trimmed.indexOf("}", i + 1);
      if (closeIndex === -1) {
        return { kind: "unmatchedOpen" };
      }
      const inner = trimmed.slice(i + 1, closeIndex);
      if (inner.includes("{")) {
        return { kind: "unmatchedOpen" };
      }
      const parsed = parseRenameTemplateTokenSpec(inner);
      if (!parsed.ok) {
        if (parsed.kind === "invalidFilter") {
          return { kind: "invalidFilter", filter: parsed.value };
        }
        if (parsed.kind === "invalidPadding") {
          return { kind: "invalidPadding", padding: parsed.value };
        }
        return { kind: "unknownToken", token: parsed.value };
      }
      if (!validTokens.has(parsed.spec.lookupName)) {
        return { kind: "unknownToken", token: parsed.spec.tokenName };
      }
      if (parsed.spec.lookupName === requiredToken) {
        sawRequiredToken = true;
      }
      i = closeIndex + 1;
    } else if (trimmed[i] === "}") {
      return { kind: "unmatchedClose" };
    } else {
      const character = trimmed[i];
      if (isIllegalFolderTemplateLiteral(character)) {
        return { kind: "illegalCharacter", character };
      }
      i++;
    }
  }

  return sawRequiredToken
    ? null
    : { kind: "missingRequiredToken", token: requiredToken ?? "" };
}

export function validateRenameTemplateSyntax(
  template: string,
  validTokens: ReadonlySet<string>,
): RenameTemplateValidationIssue | null {
  if (!template.trim()) {
    return { kind: "empty" };
  }

  let i = 0;
  let escapedLiteralOpenCount = 0;
  while (i < template.length) {
    if (template.startsWith("{{", i)) {
      escapedLiteralOpenCount += 1;
      i += 2;
      continue;
    }
    if (template.startsWith("}}", i)) {
      if (escapedLiteralOpenCount > 0) {
        escapedLiteralOpenCount -= 1;
      }
      i += 2;
      continue;
    }
    if (template[i] === "{") {
      const closeIndex = template.indexOf("}", i + 1);
      if (closeIndex === -1) {
        return { kind: "unmatchedOpen" };
      }
      const inner = template.slice(i + 1, closeIndex);
      if (inner.includes("{")) {
        return { kind: "unmatchedOpen" };
      }
      const parsed = parseRenameTemplateTokenSpec(inner);
      if (!parsed.ok) {
        if (parsed.kind === "invalidFilter") {
          return { kind: "invalidFilter", filter: parsed.value };
        }
        if (parsed.kind === "invalidPadding") {
          return { kind: "invalidPadding", padding: parsed.value };
        }
        return { kind: "unknownToken", token: parsed.value };
      }
      if (!validTokens.has(parsed.spec.lookupName)) {
        return { kind: "unknownToken", token: parsed.spec.tokenName };
      }
      i = closeIndex + 1;
    } else if (template[i] === "}") {
      if (escapedLiteralOpenCount > 0) {
        escapedLiteralOpenCount -= 1;
        i++;
        continue;
      }
      return { kind: "unmatchedClose" };
    } else {
      i++;
    }
  }

  return null;
}

export function applyRenameTemplatePreview(
  template: string,
  validTokens: ReadonlySet<string>,
  sampleValues: Record<string, string>,
): string | null {
  if (!template.trim()) {
    return null;
  }

  let result = "";
  let i = 0;
  let escapedLiteralOpenCount = 0;
  while (i < template.length) {
    if (template.startsWith("{{", i)) {
      result += "{";
      escapedLiteralOpenCount += 1;
      i += 2;
      continue;
    }
    if (template.startsWith("}}", i)) {
      result += "}";
      if (escapedLiteralOpenCount > 0) {
        escapedLiteralOpenCount -= 1;
      }
      i += 2;
      continue;
    }
    if (template[i] === "{") {
      const closeIndex = template.indexOf("}", i + 1);
      if (closeIndex === -1) return null;
      const inner = template.slice(i + 1, closeIndex);
      if (inner.includes("{")) return null;
      const parsed = parseRenameTemplateTokenSpec(inner);
      if (!parsed.ok || !validTokens.has(parsed.spec.lookupName)) return null;
      let value = sampleValues[parsed.spec.lookupName] ?? "";
      if (parsed.spec.padWidth > 0 && /^\d+$/.test(value)) {
        value = value.padStart(parsed.spec.padWidth, "0");
      }
      result += applyRenameTokenFilters(value, parsed.spec.filters);
      i = closeIndex + 1;
    } else if (template[i] === "}") {
      if (escapedLiteralOpenCount > 0) {
        result += "}";
        escapedLiteralOpenCount -= 1;
        i++;
        continue;
      }
      return null;
    } else {
      result += template[i];
      i++;
    }
  }

  return result;
}

export function splitRenameTemplateSegments(
  template: string,
  validTokens: ReadonlySet<string>,
): RenameTemplateSegment[] {
  if (!template) {
    return [];
  }

  const segments: RenameTemplateSegment[] = [];
  let plain = "";
  let cursor = 0;

  const pushPlain = () => {
    if (plain.length === 0) {
      return;
    }
    segments.push({ text: plain, isToken: false });
    plain = "";
  };

  while (cursor < template.length) {
    if (template.startsWith("{{", cursor) || template.startsWith("}}", cursor)) {
      plain += template.slice(cursor, cursor + 2);
      cursor += 2;
      continue;
    }

    if (template[cursor] === "{") {
      const closeIndex = template.indexOf("}", cursor + 1);
      if (closeIndex !== -1) {
        const inner = template.slice(cursor + 1, closeIndex);
        const parsed = inner.includes("{")
          ? null
          : parseRenameTemplateTokenSpec(inner);
        if (parsed?.ok && validTokens.has(parsed.spec.lookupName)) {
          pushPlain();
          segments.push({
            text: template.slice(cursor, closeIndex + 1),
            isToken: true,
          });
          cursor = closeIndex + 1;
          continue;
        }
      }
    }

    plain += template[cursor];
    cursor++;
  }

  pushPlain();
  return segments;
}

export function parseRenameTemplateTokenSpec(inner: string): RenameTokenParseResult {
  const parts = inner.split("|");
  const tokenCore = parts.shift()?.trim() ?? "";
  if (!tokenCore) {
    return { ok: false, kind: "emptyToken", value: inner };
  }

  const colonIdx = tokenCore.indexOf(":");
  const tokenName = (colonIdx >= 0 ? tokenCore.slice(0, colonIdx) : tokenCore).trim();
  if (!tokenName) {
    return { ok: false, kind: "emptyToken", value: inner };
  }

  let padWidth = 0;
  if (colonIdx >= 0) {
    const rawPadding = tokenCore.slice(colonIdx + 1).trim();
    if (!/^\d+$/.test(rawPadding)) {
      return { ok: false, kind: "invalidPadding", value: rawPadding };
    }
    padWidth = Number(rawPadding);
    if (!Number.isSafeInteger(padWidth) || padWidth > MAX_RENAME_TEMPLATE_PADDING_WIDTH) {
      return { ok: false, kind: "invalidPadding", value: rawPadding };
    }
  }
  const filters: RenameTokenFilter[] = [];

  for (const rawFilter of parts) {
    const filter = rawFilter.trim();
    if (filter.startsWith(SPACE_FILTER_PREFIX)) {
      const replacement = filter.slice(SPACE_FILTER_PREFIX.length);
      if (!VALID_SPACE_REPLACEMENTS.has(replacement)) {
        return { ok: false, kind: "invalidFilter", value: filter };
      }
      filters.push({ kind: "space", replacement });
      continue;
    }

    if (filter.startsWith(TRUNCATE_FILTER_PREFIX)) {
      const rawLimit = filter.slice(TRUNCATE_FILTER_PREFIX.length);
      if (!/^\d+$/.test(rawLimit)) {
        return { ok: false, kind: "invalidFilter", value: filter };
      }
      const limit = Number.parseInt(rawLimit, 10);
      if (limit <= 0) {
        return { ok: false, kind: "invalidFilter", value: filter };
      }
      filters.push({ kind: "truncate", limit });
      continue;
    }

    return { ok: false, kind: "invalidFilter", value: filter };
  }

  return {
    ok: true,
    spec: {
      tokenName,
      lookupName: tokenName.toLowerCase(),
      padWidth,
      filters,
    },
  };
}

export function applyRenameTokenFilters(value: string, filters: RenameTokenFilter[]): string {
  return filters.reduce((current, filter) => {
    switch (filter.kind) {
      case "space":
        return current.replace(/\s/g, filter.replacement);
      case "truncate":
        return Array.from(current).slice(0, filter.limit).join("");
      default:
        return current;
    }
  }, value);
}

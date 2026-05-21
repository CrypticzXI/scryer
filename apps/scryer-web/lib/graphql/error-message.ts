import type { CombinedError } from "urql";

const GRAPHQL_PREFIX_PATTERNS: RegExp[] = [
  /^\[GraphQL\]\s*/i,
  /^repository:\s*/i,
  /^validation:\s*/i,
];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function firstGraphQlErrorMessage(error: CombinedError | unknown): string | null {
  if (!isRecord(error) || !Array.isArray(error.graphQLErrors)) {
    return null;
  }

  for (const graphQlError of error.graphQLErrors) {
    if (isRecord(graphQlError) && typeof graphQlError.message === "string") {
      const message = graphQlError.message.trim();
      if (message) {
        return message;
      }
    }
  }

  return null;
}

function fallbackErrorMessage(error: unknown): string | null {
  if (error instanceof Error) {
    const message = error.message.trim();
    return message || null;
  }

  if (isRecord(error) && typeof error.message === "string") {
    const message = error.message.trim();
    return message || null;
  }

  return null;
}

export function normalizeGraphQlErrorMessage(message: string): string {
  let normalized = message.trim();
  let changed = true;

  while (normalized && changed) {
    changed = false;
    for (const pattern of GRAPHQL_PREFIX_PATTERNS) {
      const next = normalized.replace(pattern, "").trimStart();
      if (next !== normalized) {
        normalized = next;
        changed = true;
      }
    }
  }

  return normalized.trim();
}

export function userFacingGraphQlErrorMessage(error: unknown, fallback: string): string {
  const rawMessage =
    firstGraphQlErrorMessage(error) ?? fallbackErrorMessage(error) ?? fallback.trim();
  const normalized = normalizeGraphQlErrorMessage(rawMessage);
  return normalized || fallback.trim();
}

import type { UiDateTimeFormat } from "@/lib/types/settings";

type DateFormatOptions = {
  fallback?: string;
  dateStyle?: Intl.DateTimeFormatOptions["dateStyle"];
};

type DateTimeFormatOptions = DateFormatOptions & {
  timeStyle?: Intl.DateTimeFormatOptions["timeStyle"];
};

const ISO_DATE_ONLY_PATTERN = /^\d{4}-\d{2}-\d{2}$/;

function browserLocale(): string | undefined {
  if (typeof document === "undefined") return undefined;
  return document.documentElement.lang || undefined;
}

function parseDate(value: string | null | undefined): Date | null {
  if (!value) return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date;
}

function pad2(value: number): string {
  return value.toString().padStart(2, "0");
}

function formatLocalDateParts(date: Date): string {
  return [
    date.getFullYear().toString().padStart(4, "0"),
    pad2(date.getMonth() + 1),
    pad2(date.getDate()),
  ].join("-");
}

function formatLocalTimeParts(date: Date): string {
  return `${pad2(date.getHours())}:${pad2(date.getMinutes())}`;
}

function invalidFallback(
  value: string | null | undefined,
  fallback: string | undefined,
): string {
  return fallback ?? value ?? "";
}

export function formatUiDate(
  value: string | null | undefined,
  format: UiDateTimeFormat,
  options: DateFormatOptions = {},
): string {
  if (!value) return options.fallback ?? "";
  if (format === "ISO24H" && ISO_DATE_ONLY_PATTERN.test(value)) return value;

  const date = parseDate(value);
  if (!date) return invalidFallback(value, options.fallback);

  if (format === "ISO24H") return formatLocalDateParts(date);

  return new Intl.DateTimeFormat(browserLocale(), {
    dateStyle: options.dateStyle ?? "medium",
  }).format(date);
}

export function formatUiTime(
  value: string | null | undefined,
  format: UiDateTimeFormat,
  options: Pick<DateTimeFormatOptions, "fallback" | "timeStyle"> = {},
): string {
  const date = parseDate(value);
  if (!date) return invalidFallback(value, options.fallback);

  if (format === "ISO24H") return formatLocalTimeParts(date);

  return new Intl.DateTimeFormat(browserLocale(), {
    timeStyle: options.timeStyle ?? "short",
  }).format(date);
}

export function formatUiDateTime(
  value: string | null | undefined,
  format: UiDateTimeFormat,
  options: DateTimeFormatOptions = {},
): string {
  const date = parseDate(value);
  if (!date) return invalidFallback(value, options.fallback);

  if (format === "ISO24H") {
    return `${formatLocalDateParts(date)} ${formatLocalTimeParts(date)}`;
  }

  return new Intl.DateTimeFormat(browserLocale(), {
    dateStyle: options.dateStyle ?? "medium",
    timeStyle: options.timeStyle ?? "short",
  }).format(date);
}

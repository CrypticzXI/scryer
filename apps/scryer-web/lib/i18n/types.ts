export type LocaleDictionary = Record<string, string>;

export function interpolate(
  template: string,
  values?: Record<string, string | number | boolean | null | undefined>,
): string {
  if (!values) {
    return template;
  }

  return Object.entries(values).reduce((result, [key, value]) => {
    return result.replaceAll(`{{${key}}}`, String(value ?? ""));
  }, template);
}

export const DEFAULT_LANGUAGE = "eng";

export type ProviderConfigValue = {
  key: string;
  label?: string | null;
  fieldType?: string | null;
  required?: boolean | null;
  defaultValue?: string | null;
  valueSource?: string | null;
  role?: string | null;
  hostBinding?: string | null;
  options?: Array<{ value: string; label: string }> | null;
  helpText?: string | null;
  stringValue?: string | null;
  boolValue?: boolean | null;
  intValue?: number | null;
  floatValue?: number | null;
  secretStored?: boolean | null;
};

export type ProviderConfigValueInput = {
  key: string;
  stringValue?: string;
  boolValue?: boolean;
  intValue?: number;
  floatValue?: number;
  secretValue?: string;
  clearSecret?: boolean;
};

export function providerConfigValuesToRecord(
  values: ProviderConfigValue[] | null | undefined,
): Record<string, string> {
  const record: Record<string, string> = {};
  for (const value of values ?? []) {
    if (typeof value.stringValue === "string") {
      record[value.key] = value.stringValue;
    } else if (typeof value.boolValue === "boolean") {
      record[value.key] = value.boolValue ? "true" : "false";
    } else if (typeof value.intValue === "number") {
      record[value.key] = String(value.intValue);
    } else if (typeof value.floatValue === "number") {
      record[value.key] = String(value.floatValue);
    }
  }
  return record;
}

export function providerConfigRecordToValues(
  record: Record<string, string> | undefined,
  secretKeys: Iterable<string> = [],
): ProviderConfigValueInput[] {
  const secretKeySet = new Set(secretKeys);
  return Object.entries(record ?? {})
    .filter(([key, value]) => key.trim() !== "" && value !== "")
    .map(([key, value]) =>
      secretKeySet.has(key)
        ? { key, secretValue: value }
        : { key, stringValue: value },
    );
}

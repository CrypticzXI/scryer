import { Check, Loader2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input, signedIntegerInputProps } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { selectorId } from "@/lib/utils/dom-ids";
import { type ConfigFieldDef, visibleIndexerConfigFields } from "@/lib/types";

interface ProviderOption {
  value: string;
  label: string;
  defaultBaseUrl?: string;
  configFields: ConfigFieldDef[];
}

interface SetupIndexerViewProps {
  t: (key: string) => string;
  name: string;
  providerType: string;
  configValues: Record<string, string>;
  providerOptions: ProviderOption[];
  onNameChange: (value: string) => void;
  onProviderTypeChange: (value: string) => void;
  onConfigValueChange: (key: string, value: string) => void;
  onTestConnection: () => void;
  onNext: () => void;
  onBack: () => void;
  onSkip?: () => void;
  testing: boolean;
  testResult: "success" | "failed" | null;
  saving: boolean;
  saved: boolean;
  error: string | null;
}

function isMissingRequiredField(
  field: ConfigFieldDef,
  configValues: Record<string, string>,
) {
  if (!field.required || field.valueSource === "host_binding") {
    return false;
  }

  const value =
    configValues[field.key] ??
    field.defaultValue ??
    (field.fieldType === "bool" ? "false" : "");
  return field.fieldType !== "bool" && value.trim() === "";
}

function DynamicConfigField({
  t,
  field,
  value,
  onChange,
}: {
  t: SetupIndexerViewProps["t"];
  field: ConfigFieldDef;
  value: string;
  onChange: (key: string, value: string) => void;
}) {
  const fieldId = selectorId("setup-indexer-field", field.key);
  const requiredMarker = field.required ? (
    <span aria-hidden="true" className="text-destructive">
      *
    </span>
  ) : null;

  if (field.fieldType === "bool") {
    return (
      <label className="flex items-center gap-2">
        <input
          id={fieldId}
          type="checkbox"
          checked={value === "true"}
          onChange={(event) =>
            onChange(field.key, event.target.checked ? "true" : "false")
          }
          className="accent-primary"
        />
        <span className="inline-flex items-center gap-2 text-sm">
          {field.label}
          {requiredMarker}
        </span>
        {field.helpText ? (
          <span className="text-xs text-muted-foreground">
            {field.helpText}
          </span>
        ) : null}
      </label>
    );
  }

  if (field.fieldType === "select" && field.options.length > 0) {
    return (
      <label className="space-y-2">
        <Label className="inline-flex items-center gap-2" htmlFor={fieldId}>
          {field.label}
          {requiredMarker}
        </Label>
        <Select
          value={value || field.defaultValue || ""}
          onValueChange={(nextValue) => onChange(field.key, nextValue)}
        >
          <SelectTrigger id={fieldId} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {field.options.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {field.helpText ? (
          <p className="text-xs text-muted-foreground">{field.helpText}</p>
        ) : null}
      </label>
    );
  }

  if (field.fieldType === "multiline") {
    return (
      <label className="space-y-2">
        <Label className="inline-flex items-center gap-2" htmlFor={fieldId}>
          {field.label}
          {requiredMarker}
        </Label>
        <Textarea
          id={fieldId}
          value={value}
          onChange={(event) => onChange(field.key, event.target.value)}
          required={field.required}
          placeholder={field.defaultValue ?? ""}
          rows={5}
        />
        {field.helpText ? (
          <p className="text-xs text-muted-foreground">{field.helpText}</p>
        ) : null}
      </label>
    );
  }

  return (
    <label className="space-y-2">
      <Label className="inline-flex items-center gap-2" htmlFor={fieldId}>
        {field.label}
        {requiredMarker}
      </Label>
      <Input
        id={fieldId}
        value={value}
        onChange={(event) => onChange(field.key, event.target.value)}
        {...(field.fieldType === "number" ? signedIntegerInputProps : {})}
        type={
          field.fieldType === "password"
            ? "password"
            : field.fieldType === "number"
              ? "number"
              : "text"
        }
        required={field.required}
        placeholder={
          field.fieldType === "password"
            ? t("form.apiKeyInputPlaceholder")
            : field.defaultValue ?? ""
        }
      />
      {field.helpText ? (
        <p className="text-xs text-muted-foreground">{field.helpText}</p>
      ) : null}
    </label>
  );
}

export function SetupIndexerView({
  t,
  name,
  providerType,
  configValues,
  providerOptions,
  onNameChange,
  onProviderTypeChange,
  onConfigValueChange,
  onTestConnection,
  onNext,
  onBack,
  onSkip,
  testing,
  testResult,
  saving,
  saved,
  error,
}: SetupIndexerViewProps) {
  const selectedProvider = providerOptions.find((p) => p.value === providerType);
  const selectedProviderFields = visibleIndexerConfigFields(
    providerType,
    (selectedProvider?.configFields ?? []).filter(
      (field) => field.valueSource !== "host_binding",
    ),
  );
  const hasMissingRequiredField = selectedProviderFields.some((field) =>
    isMissingRequiredField(field, configValues),
  );
  const canTest =
    name.trim().length > 0 && providerType.length > 0 && !hasMissingRequiredField;
  const canProceed = saved;

  return (
    <div id="setup-indexer-view" className="flex flex-col gap-6">
      <div className="text-center">
        <h2 className="text-xl font-semibold">{t("setup.indexerTitle")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("setup.indexerDescription")}</p>
      </div>
      <div className="mx-auto flex w-full max-w-md flex-col gap-4">
        <div className="space-y-2">
          <Label htmlFor="setup-indexer-name">{t("label.name")}</Label>
          <Input
            id="setup-indexer-name"
            value={name}
            onChange={(e) => onNameChange(e.target.value)}
            placeholder="My Indexer"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="setup-indexer-provider">{t("settings.indexerProvider")}</Label>
          <Select value={providerType} onValueChange={onProviderTypeChange}>
            <SelectTrigger id="setup-indexer-provider"><SelectValue placeholder="Select provider" /></SelectTrigger>
            <SelectContent>
              {providerOptions.map((opt) => (
                <SelectItem key={opt.value} value={opt.value}>
                  {opt.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        {selectedProviderFields
          .filter((field) => field.fieldType !== "bool")
          .map((field) => (
            <DynamicConfigField
              key={field.key}
              t={t}
              field={field}
              value={
                configValues[field.key] ??
                field.defaultValue ??
                ""
              }
              onChange={onConfigValueChange}
            />
          ))}
        {selectedProviderFields.some((field) => field.fieldType === "bool") ? (
          <div className="flex flex-wrap items-center gap-4">
            {selectedProviderFields
              .filter((field) => field.fieldType === "bool")
              .map((field) => (
                <DynamicConfigField
                  key={field.key}
                  t={t}
                  field={field}
                  value={
                    configValues[field.key] ??
                    field.defaultValue ??
                    "false"
                  }
                  onChange={onConfigValueChange}
                />
              ))}
          </div>
        ) : null}
        <div className="flex items-center gap-3">
          <Button
            id="setup-indexer-test-connection"
            variant="outline"
            onClick={onTestConnection}
            disabled={!canTest || testing || saving}
          >
            {testing ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : null}
            {t("label.testConnection")}
          </Button>
          {testResult === "success" && (
            <span className="flex items-center gap-1 text-sm text-emerald-500">
              <Check className="h-4 w-4" /> {t("setup.connectionSuccess")}
            </span>
          )}
          {testResult === "failed" && (
            <span className="flex items-center gap-1 text-sm text-destructive">
              <X className="h-4 w-4" /> {t("setup.connectionFailed")}
            </span>
          )}
        </div>
        {error && <p id="setup-indexer-error" className="text-sm text-destructive">{error}</p>}
        {saved && (
          <p id="setup-indexer-saved" className="text-sm text-emerald-500">{t("setup.saved")}</p>
        )}
      </div>
      <div className="flex items-center justify-between pt-2">
        <Button id="setup-indexer-back" variant="ghost" onClick={onBack}>{t("setup.back")}</Button>
        <div className="flex items-center gap-3">
          {onSkip && (
            <button id="setup-indexer-skip" type="button" onClick={onSkip} className="text-sm text-muted-foreground underline-offset-4 hover:underline">
              {t("setup.skip")}
            </button>
          )}
          <Button id="setup-indexer-next" onClick={onNext} disabled={!canProceed || saving}>
            {saving ? t("label.saving") : t("setup.next")}
          </Button>
        </div>
      </div>
    </div>
  );
}

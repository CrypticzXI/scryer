import { Button } from "@/components/ui/button";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Loader2 } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import type { AcquisitionSettings } from "@/lib/types/settings";

type Props = {
  settings: AcquisitionSettings;
  setSettings: (s: AcquisitionSettings) => void;
  saving: boolean;
  loading: boolean;
  onSave: () => void;
};

export function SettingsAcquisitionSection({
  settings,
  setSettings,
  saving,
  loading,
  onSave,
}: Props) {
  const t = useTranslate();
  const update = (patch: Partial<AcquisitionSettings>) =>
    setSettings({ ...settings, ...patch });
  const parseIntegerInput = (raw: string) => {
    const nextValue = sanitizeDigits(raw);
    return nextValue === "" ? 0 : Number(nextValue);
  };

  if (loading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("label.loading")}
      </div>
    );
  }

  return (
    <div id="settings-acquisition-section" className="space-y-6 text-sm">
      <div className="flex items-center gap-3">
        <Label htmlFor="settings-acquisition-enabled">{t("settings.acq.enabled")}</Label>
        <button
          id="settings-acquisition-enabled"
          type="button"
          role="switch"
          aria-checked={settings.enabled}
          className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${settings.enabled ? "bg-primary" : "bg-muted"}`}
          onClick={() => update({ enabled: !settings.enabled })}
        >
          <span
            className={`pointer-events-none inline-block h-5 w-5 rounded-full bg-background shadow-lg transition-transform ${settings.enabled ? "translate-x-5" : "translate-x-0"}`}
          />
        </button>
      </div>

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <div className="space-y-1">
          <Label htmlFor="settings-acquisition-cooldown-hours">
            {t("settings.acq.cooldownHours")}
          </Label>
          <Input
            id="settings-acquisition-cooldown-hours"
            {...integerInputProps}
            value={settings.upgradeCooldownHours}
            onChange={(e) => update({ upgradeCooldownHours: parseIntegerInput(e.target.value) })}
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor="settings-acquisition-same-tier-delta">
            {t("settings.acq.sameTierDelta")}
          </Label>
          <Input
            id="settings-acquisition-same-tier-delta"
            {...integerInputProps}
            value={settings.sameTierMinDelta}
            onChange={(e) => update({ sameTierMinDelta: parseIntegerInput(e.target.value) })}
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor="settings-acquisition-cross-tier-delta">
            {t("settings.acq.crossTierDelta")}
          </Label>
          <Input
            id="settings-acquisition-cross-tier-delta"
            {...integerInputProps}
            value={settings.crossTierMinDelta}
            onChange={(e) => update({ crossTierMinDelta: parseIntegerInput(e.target.value) })}
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor="settings-acquisition-forced-bypass-delta">
            {t("settings.acq.forcedBypassDelta")}
          </Label>
          <Input
            id="settings-acquisition-forced-bypass-delta"
            {...integerInputProps}
            value={settings.forcedUpgradeDeltaBypass}
            onChange={(e) =>
              update({ forcedUpgradeDeltaBypass: parseIntegerInput(e.target.value) })
            }
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor="settings-acquisition-poll-interval">
            {t("settings.acq.pollInterval")}
          </Label>
          <Input
            id="settings-acquisition-poll-interval"
            {...integerInputProps}
            value={settings.pollIntervalSeconds}
            onChange={(e) => update({ pollIntervalSeconds: parseIntegerInput(e.target.value) })}
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor="settings-acquisition-sync-interval">
            {t("settings.acq.syncInterval")}
          </Label>
          <Input
            id="settings-acquisition-sync-interval"
            {...integerInputProps}
            value={settings.syncIntervalSeconds}
            onChange={(e) => update({ syncIntervalSeconds: parseIntegerInput(e.target.value) })}
          />
        </div>
        <div className="space-y-1">
          <Label htmlFor="settings-acquisition-batch-size">
            {t("settings.acq.batchSize")}
          </Label>
          <Input
            id="settings-acquisition-batch-size"
            {...integerInputProps}
            value={settings.batchSize}
            onChange={(e) => update({ batchSize: parseIntegerInput(e.target.value) })}
          />
        </div>
      </div>

      <Button id="settings-acquisition-save" onClick={onSave} disabled={saving}>
        {saving ? (
          <>
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            {t("label.saving")}
          </>
        ) : (
          t("settings.save")
        )}
      </Button>
    </div>
  );
}

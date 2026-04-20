import * as React from "react";
import {
  ClipboardList,
  Eye,
  EyeOff,
  Edit,
  Loader2,
  RefreshCw,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";

type ActionButtonProps = {
  label: string;
  icon: React.ComponentType<{ className?: string }>;
  active?: boolean;
  loading?: boolean;
  destructive?: boolean;
  disabled?: boolean;
  onClick?: () => void;
};

type Props = {
  monitored: boolean;
  monitoredUpdating?: boolean;
  refreshAndScanLoading?: boolean;
  onToggleMonitoring?: () => void;
  onRefreshAndScan?: () => void;
  onHistory?: () => void;
  settingsPanel?: React.ReactNode;
};

function ActionButton({
  label,
  icon: Icon,
  active = false,
  loading = false,
  destructive = false,
  disabled = false,
  onClick,
}: ActionButtonProps) {
  return (
    <Button
      type="button"
      variant="ghost"
      className={cn(
        "h-[84px] rounded-none border-0 bg-card/85 px-3 py-3 text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground",
        "flex flex-col items-center justify-center gap-2",
        active && "bg-accent/45 text-foreground",
        destructive && "hover:bg-destructive/10 hover:text-destructive",
      )}
      disabled={disabled || loading}
      onClick={onClick}
    >
      {loading ? (
        <Loader2 className="size-8 animate-spin" />
      ) : (
        <Icon className="size-8" />
      )}
      <span className="text-center text-[11px] font-semibold uppercase tracking-[0.14em]">
        {label}
      </span>
    </Button>
  );
}

export function OverviewControlPanel({
  monitored,
  monitoredUpdating = false,
  refreshAndScanLoading = false,
  onToggleMonitoring,
  onRefreshAndScan,
  onHistory,
  settingsPanel,
}: Props) {
  const t = useTranslate();
  const [expandedPanel, setExpandedPanel] = React.useState<"settings" | null>(null);

  const handleToggleSettings = React.useCallback(() => {
    setExpandedPanel((current) => (current === "settings" ? null : "settings"));
  }, []);

  return (
    <Card className="overflow-hidden p-0">
      <CardContent className="space-y-0 p-0">
        <div
          className={cn(
            "grid grid-cols-2 gap-px bg-border/70 sm:grid-cols-4 lg:grid-cols-4",
          )}
        >
          <ActionButton
            label={monitored ? t("title.unmonitorAction") : t("title.monitorAction")}
            icon={monitored ? EyeOff : Eye}
            active={monitored}
            destructive={monitored}
            loading={monitoredUpdating}
            disabled={!onToggleMonitoring}
            onClick={onToggleMonitoring}
          />
          <ActionButton
            label={t("title.refreshAndScanAction")}
            icon={RefreshCw}
            loading={refreshAndScanLoading}
            disabled={!onRefreshAndScan}
            onClick={onRefreshAndScan}
          />
          <ActionButton
            label={t("activity.history")}
            icon={ClipboardList}
            disabled={!onHistory}
            onClick={onHistory}
          />
          <ActionButton
            label={t("label.edit")}
            icon={Edit}
            active={expandedPanel === "settings"}
            disabled={!settingsPanel}
            onClick={handleToggleSettings}
          />
        </div>

        {expandedPanel === "settings" && settingsPanel ? (
          <div className="border-t border-border bg-card/70">
            {settingsPanel}
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}

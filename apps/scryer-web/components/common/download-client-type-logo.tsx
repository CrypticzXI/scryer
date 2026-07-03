import { PluginLogo } from "@/components/common/plugin-visual";

export function DownloadClientTypeLogo({
  typeValue,
  className = "h-4 w-4",
}: {
  typeValue: string;
  className?: string;
}) {
  return (
    <PluginLogo
      providerType={typeValue}
      pluginType="download_client"
      className={className}
    />
  );
}

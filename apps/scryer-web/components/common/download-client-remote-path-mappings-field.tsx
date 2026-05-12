import {
  LocalRemotePathMappingsField,
  type LocalRemotePathMappingsFieldProps,
} from "@/components/common/local-remote-path-mappings-field";

type DownloadClientRemotePathMappingsFieldProps = Omit<
  LocalRemotePathMappingsFieldProps,
  "direction"
>;

export function DownloadClientRemotePathMappingsField(
  props: DownloadClientRemotePathMappingsFieldProps,
) {
  return <LocalRemotePathMappingsField {...props} direction="remote-to-local" />;
}

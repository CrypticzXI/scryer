// Authored preview for the Scryer <Textarea>.
import { Textarea, Label } from "scryer-web";

export function States() {
  return (
    <div className="flex flex-col gap-3 p-6 max-w-sm">
      <Textarea placeholder="Release notes…" />
      <Textarea defaultValue={"Block releases over 100 GiB\nReject x265 unless 2160p"} />
      <Textarea placeholder="Disabled" disabled />
    </div>
  );
}

export function WithLabel() {
  return (
    <div className="flex flex-col gap-2 p-6 max-w-sm">
      <Label htmlFor="custom-script">Custom post-processing script</Label>
      <Textarea
        id="custom-script"
        defaultValue={"#!/usr/bin/env bash\n/usr/local/bin/post-process.sh \"$1\""}
      />
    </div>
  );
}

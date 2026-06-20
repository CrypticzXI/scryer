// Authored preview for the Scryer <Checkbox> (radix; checked = emerald).
import { Checkbox, Label } from "scryer-web";

export function States() {
  return (
    <div className="flex items-center gap-6 p-6">
      <Checkbox aria-label="unchecked" />
      <Checkbox defaultChecked aria-label="checked" />
      <Checkbox checked="indeterminate" aria-label="indeterminate" />
      <Checkbox defaultChecked disabled aria-label="checked disabled" />
      <Checkbox disabled aria-label="disabled" />
    </div>
  );
}

export function SettingsList() {
  return (
    <div className="flex flex-col gap-3 p-6">
      <div className="flex items-center gap-2">
        <Checkbox id="monitor" defaultChecked />
        <Label htmlFor="monitor">Monitor for new releases</Label>
      </div>
      <div className="flex items-center gap-2">
        <Checkbox id="search" defaultChecked />
        <Label htmlFor="search">Search on add</Label>
      </div>
      <div className="flex items-center gap-2">
        <Checkbox id="season-folder" />
        <Label htmlFor="season-folder">Use season folders</Label>
      </div>
    </div>
  );
}

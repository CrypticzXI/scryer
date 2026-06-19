// Authored preview for the Scryer <Label> — shown composed with the controls it labels.
import { Label, Input, Checkbox } from "scryer-web";

export function FormFields() {
  return (
    <div className="flex flex-col gap-4 p-6 max-w-sm">
      <div className="flex flex-col gap-2">
        <Label htmlFor="name">Profile name</Label>
        <Input id="name" placeholder="HD - 1080p" />
      </div>
      <div className="flex flex-col gap-2">
        <Label htmlFor="folder">Root folder</Label>
        <Input id="folder" defaultValue="/media/movies" />
      </div>
    </div>
  );
}

export function WithCheckbox() {
  return (
    <div className="flex items-center gap-2 p-6">
      <Checkbox id="upgrade" defaultChecked />
      <Label htmlFor="upgrade">Upgrade until cutoff is met</Label>
    </div>
  );
}

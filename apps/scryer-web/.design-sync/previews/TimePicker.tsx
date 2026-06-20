// Authored preview for the Scryer <TimePicker> (controlled; renders the trigger button —
// the dropdown's open state isn't externally controllable, see NOTES.md).
import { TimePicker, Label } from "scryer-web";

const noop = () => {};

export function Default() {
  return (
    <div className="flex flex-col gap-3 p-6 max-w-xs">
      <TimePicker value="20:00" onChange={noop} />
      <TimePicker value="06:30" onChange={noop} disabled />
    </div>
  );
}

export function WithLabel() {
  return (
    <div className="flex flex-col gap-2 p-6 max-w-xs">
      <Label htmlFor="rss-time">Run RSS sync at</Label>
      <TimePicker id="rss-time" value="03:15" onChange={noop} />
    </div>
  );
}

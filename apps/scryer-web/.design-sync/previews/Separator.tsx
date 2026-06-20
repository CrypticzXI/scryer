// Authored preview for the Scryer <Separator> (horizontal + vertical).
import { Separator } from "scryer-web";

export function Horizontal() {
  return (
    <div className="flex flex-col gap-3 p-6 max-w-sm">
      <div className="text-sm font-medium text-foreground">Library</div>
      <Separator />
      <div className="text-sm text-muted-foreground">1,284 movies · 312 series</div>
    </div>
  );
}

export function Vertical() {
  return (
    <div className="flex h-6 items-center gap-3 p-6 text-sm text-muted-foreground">
      <span>Movies</span>
      <Separator orientation="vertical" />
      <span>Series</span>
      <Separator orientation="vertical" />
      <span>Calendar</span>
    </div>
  );
}

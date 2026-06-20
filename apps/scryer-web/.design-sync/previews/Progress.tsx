// Authored preview for the Scryer <Progress> (determinate values; indeterminate animates
// so it isn't captured statically — noted in NOTES.md).
import { Progress } from "scryer-web";

export function Values() {
  return (
    <div className="flex flex-col gap-4 p-6 max-w-sm">
      <Progress value={25} />
      <Progress value={50} />
      <Progress value={75} />
      <Progress value={100} />
    </div>
  );
}

export function Download() {
  return (
    <div className="flex flex-col gap-2 p-6 max-w-sm">
      <div className="flex items-center justify-between text-sm">
        <span className="text-foreground">Dune.Part.Two.2160p.mkv</span>
        <span className="text-muted-foreground">67%</span>
      </div>
      <Progress value={67} />
    </div>
  );
}

// Authored preview for the Scryer <Skeleton> loading placeholders.
import { Skeleton } from "scryer-web";

export function Shapes() {
  return (
    <div className="flex items-center gap-4 p-6">
      <Skeleton className="size-12 rounded-full" />
      <div className="flex flex-col gap-2">
        <Skeleton className="h-4 w-48" />
        <Skeleton className="h-4 w-32" />
      </div>
    </div>
  );
}

export function MovieRow() {
  return (
    <div className="flex flex-col gap-3 p-6 max-w-md">
      {[0, 1, 2].map((i) => (
        <div key={i} className="flex items-center gap-3">
          <Skeleton className="h-16 w-12 rounded-md" />
          <div className="flex flex-1 flex-col gap-2">
            <Skeleton className="h-4 w-2/3" />
            <Skeleton className="h-3 w-1/3" />
          </div>
        </div>
      ))}
    </div>
  );
}

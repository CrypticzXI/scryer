// Authored preview for the Scryer <Collapsible> (rendered open so content is visible).
import { Collapsible, CollapsibleTrigger, CollapsibleContent, Button } from "scryer-web";

const Chevron = () => (
  <svg viewBox="0 0 24 24" className="size-4" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <path d="m6 9 6 6 6-6" />
  </svg>
);

export function Open() {
  return (
    <div className="p-6 max-w-sm">
      <Collapsible defaultOpen>
        <CollapsibleTrigger asChild>
          <Button variant="ghost" className="w-full justify-between">
            Advanced settings<Chevron />
          </Button>
        </CollapsibleTrigger>
        <CollapsibleContent className="mt-2 flex flex-col gap-2 text-sm text-muted-foreground">
          <div className="rounded-md border border-border px-3 py-2">Minimum availability: Released</div>
          <div className="rounded-md border border-border px-3 py-2">Quality profile: HD - 1080p</div>
          <div className="rounded-md border border-border px-3 py-2">Tags: 4k, remux</div>
        </CollapsibleContent>
      </Collapsible>
    </div>
  );
}

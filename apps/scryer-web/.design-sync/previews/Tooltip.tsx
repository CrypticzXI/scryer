// Authored preview for the Scryer <Tooltip> — wrapped in TooltipProvider and rendered open.
import { TooltipProvider, Tooltip, TooltipTrigger, TooltipContent, Button } from "scryer-web";

const Refresh = () => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <path d="M3 12a9 9 0 0 1 15-6.7L21 8" /><path d="M21 3v5h-5" /><path d="M21 12a9 9 0 0 1-15 6.7L3 16" /><path d="M3 21v-5h5" />
  </svg>
);

export function Default() {
  return (
    <TooltipProvider>
      <div className="flex justify-center pt-12">
        <Tooltip defaultOpen>
          <TooltipTrigger asChild>
            <Button variant="outline" size="icon" aria-label="Refresh"><Refresh /></Button>
          </TooltipTrigger>
          <TooltipContent>Refresh library</TooltipContent>
        </Tooltip>
      </div>
    </TooltipProvider>
  );
}

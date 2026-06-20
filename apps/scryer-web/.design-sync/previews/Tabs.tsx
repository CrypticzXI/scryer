// Authored preview for the Scryer <Tabs> (active tab = emerald).
import { Tabs, TabsList, TabsTrigger, TabsContent } from "scryer-web";

export function Default() {
  return (
    <div className="p-6 max-w-lg">
      <Tabs defaultValue="overview">
        <TabsList>
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="files">Files</TabsTrigger>
          <TabsTrigger value="history">History</TabsTrigger>
        </TabsList>
        <TabsContent value="overview">
          <div className="rounded-md border border-border p-3 text-sm text-muted-foreground">
            Dune: Part Two (2024) · 2160p · monitored · 18.4 GB on disk.
          </div>
        </TabsContent>
        <TabsContent value="files">
          <div className="rounded-md border border-border p-3 text-sm text-muted-foreground">
            1 file · Dune.Part.Two.2024.2160p.BluRay.x265.mkv
          </div>
        </TabsContent>
        <TabsContent value="history">
          <div className="rounded-md border border-border p-3 text-sm text-muted-foreground">
            Grabbed from NZBgeek · imported 3 days ago.
          </div>
        </TabsContent>
      </Tabs>
    </div>
  );
}

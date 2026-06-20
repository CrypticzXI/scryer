// Authored preview for the Scryer <Card> family (Card / CardHeader / CardTitle / CardContent).
// Text-heavy on purpose — exercises the Space Grotesk heading + Inter body fonts.
import { Card, CardHeader, CardTitle, CardContent, Button, Separator } from "scryer-web";

export function Basic() {
  return (
    <div className="p-6 max-w-md">
      <Card>
        <CardHeader>
          <CardTitle>Quality Profile</CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">
            Releases are upgraded until they reach your cutoff. Higher-ranked
            qualities are preferred when multiple releases are available.
          </p>
        </CardContent>
      </Card>
    </div>
  );
}

export function Stats() {
  return (
    <div className="grid grid-cols-3 gap-4 p-6">
      {[
        { label: "Movies", value: "1,284" },
        { label: "Monitored", value: "1,107" },
        { label: "Missing", value: "63" },
      ].map((s) => (
        <Card key={s.label}>
          <CardContent>
            <div className="text-2xl font-semibold text-foreground">{s.value}</div>
            <div className="text-xs text-muted-foreground mt-1">{s.label}</div>
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

export function WithActions() {
  return (
    <div className="p-6 max-w-md">
      <Card>
        <CardHeader>
          <CardTitle>Download Client</CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted-foreground">
            qBittorrent · http://localhost:8080 · connected
          </p>
          <Separator />
          <div className="flex items-center gap-2">
            <Button size="sm">Test</Button>
            <Button size="sm" variant="outline">Edit</Button>
            <Button size="sm" variant="ghost">Remove</Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

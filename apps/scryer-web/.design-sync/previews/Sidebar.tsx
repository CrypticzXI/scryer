// Authored preview for the Scryer <Sidebar> — wrapped in SidebarProvider, rendered as a
// static panel (collapsible="none") so the nav is visible in the card.
import { SidebarProvider, Sidebar, SidebarHeader, SidebarContent, SidebarGroup, SidebarGroupLabel, SidebarMenu, SidebarMenuItem, SidebarMenuButton } from "scryer-web";

const Icon = ({ d }: { d: string }) => (
  <svg viewBox="0 0 24 24" className="size-4" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <path d={d} />
  </svg>
);

export function Navigation() {
  return (
    <SidebarProvider>
      <Sidebar collapsible="none">
        <SidebarHeader>
          <div className="px-2 py-1.5 text-base font-semibold text-foreground">Scryer</div>
        </SidebarHeader>
        <SidebarContent>
          <SidebarGroup>
            <SidebarGroupLabel>Library</SidebarGroupLabel>
            <SidebarMenu>
              <SidebarMenuItem>
                <SidebarMenuButton isActive>
                  <Icon d="m4 8 8-5 8 5v11a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1Z" />Movies
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <SidebarMenuButton>
                  <Icon d="M2 7h20v12H2zM8 3l4 4 4-4" />Series
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <SidebarMenuButton>
                  <Icon d="M8 2v4M16 2v4M3 10h18M5 4h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2Z" />Calendar
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                <SidebarMenuButton>
                  <Icon d="M22 12h-4l-3 9L9 3l-3 9H2" />Activity
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroup>
          <SidebarGroup>
            <SidebarGroupLabel>System</SidebarGroupLabel>
            <SidebarMenu>
              <SidebarMenuItem>
                <SidebarMenuButton>
                  <Icon d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm8-3a8 8 0 0 0-.1-1l2-1.6-2-3.4-2.3 1a8 8 0 0 0-1.7-1l-.4-2.5H9.5L9 4.5a8 8 0 0 0-1.7 1l-2.3-1-2 3.4L5 9a8 8 0 0 0 0 2l-2 1.6 2 3.4 2.3-1a8 8 0 0 0 1.7 1l.4 2.5h4l.4-2.5a8 8 0 0 0 1.7-1l2.3 1 2-3.4L19 13c.1-.3.1-.7.1-1Z" />Settings
                </SidebarMenuButton>
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroup>
        </SidebarContent>
      </Sidebar>
    </SidebarProvider>
  );
}

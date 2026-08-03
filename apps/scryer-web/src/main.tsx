import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "react-router/dom";
import { Provider as UrqlProvider } from "urql";
import { ThemeProvider } from "next-themes";
import { backendClient } from "@/lib/graphql/urql-client";
import { SELECTABLE_THEMES } from "@/lib/theme";
import { UiSettingsProvider } from "@/lib/context/ui-settings-context";

import "@fontsource-variable/inter";
import "@fontsource-variable/space-grotesk";

import "@/app/globals.css";

import { registerServiceWorker } from "@/lib/pwa/register-service-worker";
import { router } from "./router";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ThemeProvider attribute="class" defaultTheme="dark" enableSystem themes={[...SELECTABLE_THEMES]}>
      <UrqlProvider value={backendClient}>
        <UiSettingsProvider>
          <RouterProvider router={router} />
        </UiSettingsProvider>
      </UrqlProvider>
    </ThemeProvider>
  </StrictMode>,
);

registerServiceWorker();

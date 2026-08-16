import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";

const WEB_ROOT = fileURLToPath(new URL("../..", import.meta.url));
const SCRYER_TOTP_URI =
  "otpauth://totp/Scryer:jen%40example.test?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=Scryer&algorithm=SHA1&digits=6&period=30";

async function renderTotpQrCode(): Promise<string> {
  const server = await createServer({
    root: WEB_ROOT,
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
  });

  try {
    const module = await server.ssrLoadModule(
      "/components/common/totp-qr-code.tsx",
    );
    const TotpQrCode = module.TotpQrCode as ComponentType<{ value: string }>;
    return renderToStaticMarkup(
      createElement(TotpQrCode, { value: SCRYER_TOTP_URI }),
    );
  } finally {
    await server.close();
  }
}

test("TOTP QR keeps its scanner-safe rendering contract", async () => {
  const markup = await renderTotpQrCode();
  const wrapperClasses = markup.match(/^<div class="([^"]+)">/)?.[1];
  const svgTag = markup.match(/<svg\b[^>]*>/)?.[0];

  assert.ok(wrapperClasses);
  assert.ok(svgTag);
  assert.ok(wrapperClasses.split(/\s+/).includes("bg-white"));
  assert.ok(wrapperClasses.split(/\s+/).includes("p-6"));
  assert.match(svgTag, /shape-rendering="crispEdges"/);
  assert.match(svgTag, /height="256"/);
  assert.match(svgTag, /width="256"/);
  assert.deepEqual(
    [...markup.matchAll(/<path\b[^>]*fill="([^"]+)"/g)].map(
      (match) => match[1],
    ),
    ["#FFFFFF", "#000000"],
  );
});

test("TOTP QR remains sparse enough for 1Password screen scanning", async () => {
  const markup = await renderTotpQrCode();
  const viewBox = markup.match(/viewBox="0 0 (\d+) (\d+)"/);

  assert.ok(viewBox);
  const moduleWidth = Number(viewBox[1]);
  const moduleHeight = Number(viewBox[2]);
  assert.equal(moduleWidth, moduleHeight);
  assert.ok(
    moduleWidth <= 41,
    `expected at most 41 modules per side, received ${moduleWidth}`,
  );
  assert.ok(
    256 / moduleWidth >= 6,
    `expected at least 6 rendered pixels per module, received ${256 / moduleWidth}`,
  );
});

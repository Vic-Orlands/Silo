import { chromium } from "playwright";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const extensionPath = path.join(root, "extension");
const context = await chromium.launchPersistentContext("", {
  headless: false,
  args: [
    `--disable-extensions-except=${extensionPath}`,
    `--load-extension=${extensionPath}`,
  ],
});

try {
  let serviceWorker = context.serviceWorkers()[0];
  if (!serviceWorker) serviceWorker = await context.waitForEvent("serviceworker", { timeout: 10000 });
  const extensionId = new URL(serviceWorker.url()).host;
  const page = await context.newPage();
  await page.goto(`chrome-extension://${extensionId}/popup.html`);
  await page.getByRole("heading", { name: "SILO" }).waitFor();
  await page.getByText("Your local vault is locked.").waitFor();
  await page.getByRole("button", { name: /Unlock Silo/ }).waitFor();
  console.log("Browser extension popup smoke test passed.");
} finally {
  await context.close();
}

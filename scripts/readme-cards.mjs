// Renders scripts/readme-cards.html to docs/readme/{hero,numbers,sponsor}-{light,dark}.png at 2x.
// Uses the Playwright installed in app/:  node scripts/readme-cards.mjs
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const { chromium } = createRequire(path.join(root, "app", "package.json"))("playwright");
const page_url = pathToFileURL(path.join(root, "scripts", "readme-cards.html")).href;
const browser = await chromium.launch();
const page = await browser.newPage({ deviceScaleFactor: 2, viewport: { width: 1100, height: 1200 } });
for (const theme of ["light", "dark"]) {
  await page.goto(`${page_url}?theme=${theme}`);
  await page.waitForLoadState("networkidle");
  for (const id of ["hero", "numbers", "sponsor"]) {
    const out = path.join(root, "docs", "readme", `${id}-${theme}.png`);
    await page.locator(`#${id}`).screenshot({ path: out, omitBackground: true });
    console.log("wrote", path.relative(root, out));
  }
}
await browser.close();

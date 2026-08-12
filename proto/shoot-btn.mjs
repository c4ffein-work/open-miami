// Live test of the on-screen trigger button.
import { chromium } from "playwright";
const BASE = "http://localhost:8097/proto/shoggoth-v2.html";
const OUT = process.env.OUT || ".";
const browser = await chromium.launch({
  args: ["--no-sandbox","--disable-setuid-sandbox","--disable-gpu",
    "--single-process","--disable-dev-shm-usage","--enable-unsafe-swiftshader"],
});
const page = await browser.newPage({ viewport: { width: 420, height: 500 } });
page.on("pageerror", e => console.log("  [pageerror]", e.message));
// masked, LIVE (no ?t) so the button/loop are active
await page.goto(BASE + "?phase=masked", { waitUntil: "load" });
await page.waitForFunction("window.__ready === true", { timeout: 8000 });
await page.waitForTimeout(400);
const canvas = page.locator("canvas#c");
await canvas.screenshot({ path: `${OUT}/shoggoth_v2_btn_before.png` });
console.log("btn text before:", await page.locator("#trigger").textContent());
await page.locator("#trigger").click();
console.log("btn text after click:", await page.locator("#trigger").textContent());
await page.waitForTimeout(1400);                     // mid-transition
await canvas.screenshot({ path: `${OUT}/shoggoth_v2_btn_mid.png` });
await page.waitForTimeout(2600);                     // fully raw
await canvas.screenshot({ path: `${OUT}/shoggoth_v2_btn_raw.png` });
const hud = await page.locator("#hud").textContent();
console.log("hud after:", hud);
await browser.close();
console.log("done");

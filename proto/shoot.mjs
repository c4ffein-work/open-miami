// Playwright screenshot driver for proto/robot.html
// Usage: bun proto/shoot.mjs   (server must be running on :8099)
import { chromium } from "playwright";

const BASE = "http://localhost:8099/proto/robot.html";
const OUT = "/home/dev/workspace/open-miami/proto/frames";

const browser = await chromium.launch({
  args: [
    "--no-sandbox","--disable-setuid-sandbox","--disable-gpu",
    "--single-process","--disable-dev-shm-usage","--enable-unsafe-swiftshader",
  ],
});
const page = await browser.newPage({ viewport: { width: 420, height: 460 } });
page.on("console", m => console.log("  [page]", m.text()));
page.on("pageerror", e => console.log("  [pageerror]", e.message));

async function shot(name, params){
  const url = BASE + "?" + params;
  await page.goto(url, { waitUntil: "load" });
  // wait for the render-ready flag the page sets after drawing
  await page.waitForFunction("window.__ready === true", { timeout: 8000 });
  await page.waitForTimeout(120); // let swiftshader flush
  const canvas = page.locator("canvas#c");
  await canvas.screenshot({ path: `${OUT}/${name}.png` });
  console.log("saved", `${OUT}/${name}.png`, "<-", params);
}

// Walk cycle: 6 frames across one cycle, coral player
for (let i = 0; i < 6; i++) {
  await shot(`walk_coral_${i}`, `pose=walk&color=coral&frame=${i*2}`);
}
// Shoot pose, coral, with recoil variants
await shot(`shoot_coral_0`, `pose=shoot&color=coral&t=0.10`);
await shot(`shoot_coral_1`, `pose=shoot&color=coral&t=0.16`);

// Rogue palettes - one walk + one shoot each
for (const c of ["red","magenta","violet"]) {
  await shot(`walk_${c}`,  `pose=walk&color=${c}&frame=3`);
  await shot(`shoot_${c}`, `pose=shoot&color=${c}&t=0.10`);
}

// A "contact sheet" style bigger pixel look to judge the ink style
await shot(`walk_coral_bigpx`, `pose=walk&color=coral&frame=3&px=8`);
await shot(`walk_coral_finepx`, `pose=walk&color=coral&frame=3&px=2`);

await browser.close();
console.log("done");

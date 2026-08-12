// Playwright screenshot driver for proto/shoggoth-v2.html
// Usage: bun proto/shoot-v2.mjs   (server must be running on :8097)
import { chromium } from "playwright";

const BASE = "http://localhost:8097/proto/shoggoth-v2.html";
const OUT = process.env.OUT || "/home/dev/workspace/open-miami/.claude/worktrees/agent-ad5319ab6f08c541d/proto/frames";

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
  await page.waitForFunction("window.__ready === true", { timeout: 8000 });
  await page.waitForTimeout(150);
  const canvas = page.locator("canvas#c");
  await canvas.screenshot({ path: `${OUT}/${name}.png` });
  console.log("saved", `${OUT}/${name}.png`, "<-", params);
}

// masked, mid-drift (a look-up-free moment) and a look-up moment.
// From the wander sim: mode0 first ~3.5-6s (drift), then a look-up window.
await shot("shoggoth_v2_masked",  "phase=masked&t=2.2");
await shot("shoggoth_v2_masked_b","phase=masked&t=1.0");
// look-up: force a frame where lookUp is high. Probe a few then keep the clearest as _lookup.
await shot("shoggoth_v2_lookup",  "phase=masked&t=7.0");
await shot("shoggoth_v2_lookup_b","phase=masked&t=7.6");
await shot("shoggoth_v2_lookup_c","phase=masked&t=6.6");

// transition frames across the mask-off
await shot("shoggoth_v2_transition_0", "phase=transition&t=0.5");
await shot("shoggoth_v2_transition_1", "phase=transition&t=1.3");
await shot("shoggoth_v2_transition_2", "phase=transition&t=2.2");
await shot("shoggoth_v2_transition_3", "phase=transition&t=3.0");

// enraged raw form
await shot("shoggoth_v2_enraged",   "phase=enraged&t=1.4");
await shot("shoggoth_v2_enraged_b", "phase=enraged&t=3.2");

// auto-cycle sampling to sanity-check the whole beat
await shot("shoggoth_v2_auto_masked", "t=3.0");
await shot("shoggoth_v2_auto_trans",  "t=10.5");
await shot("shoggoth_v2_auto_enraged","t=14.0");

await browser.close();
console.log("done");

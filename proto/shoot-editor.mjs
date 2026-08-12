// Playwright screenshot driver for proto/editor.html
// Usage (from repo root, server on :8096):
//   bun proto/shoot-editor.mjs
import { chromium } from "playwright";

const ROOT = "http://localhost:8096";
const BASE = ROOT + "/proto/editor.html";
const OUT  = new URL("./frames", import.meta.url).pathname;

const browser = await chromium.launch({
  args: [
    "--no-sandbox","--disable-setuid-sandbox","--disable-gpu",
    "--single-process","--disable-dev-shm-usage","--enable-unsafe-swiftshader",
  ],
});
const page = await browser.newPage({ viewport: { width: 900, height: 760 } });
page.on("console", m => console.log("  [page]", m.text()));
page.on("pageerror", e => console.log("  [pageerror]", e.message));

await page.goto(BASE, { waitUntil: "load" });
await page.waitForFunction("window.__ready === true", { timeout: 8000 });

// Helper: apply editor state, let a few frames run, then shoot a target.
async function apply(patch){
  await page.evaluate(p => window.__editor.set(p), patch);
  await page.waitForTimeout(220);
}
async function shotEl(sel, name){
  await page.locator(sel).screenshot({ path: `${OUT}/${name}.png` });
  console.log("saved", `${OUT}/${name}.png`);
}
async function shotPage(name){
  await page.screenshot({ path: `${OUT}/${name}.png` });
  console.log("saved", `${OUT}/${name}.png`);
}

// 1) Full editor layout (both viewports + controls), idle/coral, gentle top-down orbit
await apply({ pose:"idle", color:"coral", px:5, facingDeg:0, yaw:0.6, pitch:0.95 });
await shotPage("editor_layout");

// 2) The 2D in-game view at several facing angles (one-axis turn)
for (const deg of [0, 45, 90, 135, 180, 270]) {
  await apply({ pose:"walk", color:"coral", facingDeg:deg });
  await shotEl("#game", `editor_2d_face_${String(deg).padStart(3,"0")}`);
}

// 3) Each pose, shown in BOTH viewports (orbit + baked 2D) via full layout
for (const pose of ["idle","walk","shoot","hit"]) {
  await apply({ pose, color:"coral", facingDeg:0, yaw:0.6, pitch:0.95 });
  await shotPage(`editor_pose_${pose}`);
  await shotEl("#orbit", `editor_orbit_${pose}`);
  await shotEl("#game",  `editor_2d_${pose}`);
}

// 4) Orbit read-check: same pose from a few camera angles
await apply({ pose:"shoot", color:"violet" });
for (const [yaw,pitch,tag] of [[0.0,0.4,"front"],[1.4,0.9,"side"],[0.6,1.35,"top"],[0.6,0.2,"low"]]) {
  await apply({ yaw, pitch });
  await shotEl("#orbit", `editor_orbit_angle_${tag}`);
}

// 5) A color sweep on the 2D bake
for (const color of ["coral","red","magenta","violet"]) {
  await apply({ pose:"shoot", color, facingDeg:30 });
  await shotEl("#game", `editor_2d_color_${color}`);
}

await browser.close();
console.log("done");

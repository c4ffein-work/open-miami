const { test, expect } = require('@playwright/test');
const {
  collectErrors,
  loadFloor,
  lastFrameTexts,
  hudValue,
  purgeRogues,
  walkFloor1ToServiceLift,
  expectFloor2,
} = require('./helpers');

// The floor/scenario engine: `?floor=N&debug` starts on a floor with the debug
// keys enabled, the entry elevator is the spawn, the debug purge (I then K)
// downs every rogue so the all-dead step opens the exit, and standing in the
// open exit extracts to the next floor. Keys are HELD (down / wait / up) so the
// wasm input layer sees them. See helpers.js.
test.describe('Open Miami - Floors & scenarios', () => {
  test('extraction elevator takes the player from floor 1 to floor 2', async ({ page }) => {
    const errors = collectErrors(page);
    const canvas = await loadFloor(page, 1);

    // Floor 1's scenario objective is drawn under the HUD.
    let texts = await lastFrameTexts(page);
    expect(texts.some((s) => s.includes('SERVICE LIFT'))).toBe(true);

    // Debug overlays on, then purge every rogue -> the SERVICE LIFT opens.
    await purgeRogues(page);
    await page.screenshot({ path: 'test-results/scenario-01-cleared.png' });

    // Floor 1: entry SE (895,750) -> exit NW (105,50).
    await walkFloor1ToServiceLift(page);
    await page.waitForTimeout(1200); // dwell (0.6s) + the completion card starts
    await page.screenshot({ path: 'test-results/scenario-02-extracting.png' });

    // Card ends -> floor 2 loads (COLD STORAGE: FREIGHT LIFT objective).
    texts = await expectFloor2(page);
    expect(Number(hudValue(texts, 'Rogues:'))).toBeGreaterThan(0);
    await page.screenshot({ path: 'test-results/scenario-03-next-floor.png' });

    await expect(canvas).toBeVisible();
    // WebGL is required: any page/console error fails the test.
    expect(errors).toEqual([]);
  });
});

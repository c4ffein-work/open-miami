const { test, expect } = require('@playwright/test');

// The floor/scenario engine: `?floor=N` starts on a floor, the entry elevator
// is the spawn, the debug purge (I then K) downs every rogue so the all-dead
// step opens the exit, and standing in the open exit extracts to the next
// floor. Keys are HELD (down / wait / up) so the wasm input layer sees them.
test.describe('Open Miami - Floors & scenarios', () => {
  async function hold(page, key, ms) {
    await page.keyboard.down(key);
    await page.waitForTimeout(ms);
    await page.keyboard.up(key);
  }

  test('extraction elevator takes the player from floor 1 to floor 2', async ({ page }) => {
    const errors = [];
    page.on('pageerror', (e) => errors.push(e.message));
    page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });

    await page.goto('/?floor=1');
    await page.waitForSelector('canvas#glcanvas', { timeout: 10000 });
    await page.waitForTimeout(2500);
    const canvas = page.locator('canvas#glcanvas');
    await expect(canvas).toBeVisible();
    await canvas.focus();

    // Debug overlays on, then purge every rogue -> the SERVICE LIFT opens.
    await hold(page, 'i', 150);
    await hold(page, 'k', 150);
    await page.waitForTimeout(1200);
    await page.screenshot({ path: 'test-results/scenario-01-cleared.png' });

    // Floor 1: entry SE (895,750) -> exit NW (105,50): west along the bottom,
    // then north into the car (player speed 200 px/s).
    await page.mouse.move(300, 400);
    await hold(page, 'a', 3950);
    await page.mouse.move(640, 100);
    await hold(page, 'w', 3700);
    await page.waitForTimeout(1200); // dwell (0.6s) + the completion card starts
    await page.screenshot({ path: 'test-results/scenario-02-extracting.png' });
    await page.waitForTimeout(3000); // card ends -> floor 2 loads
    await page.screenshot({ path: 'test-results/scenario-03-next-floor.png' });

    await expect(canvas).toBeVisible();
    const critical = errors.filter((e) => !e.includes('WebGL') && !e.includes('WEBGL'));
    expect(critical).toEqual([]);
  });
});

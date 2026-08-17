const { test, expect } = require('@playwright/test');
const {
  hold,
  collectErrors,
  loadFloor,
  lastFrameTexts,
  hudValue,
  playerPos,
  purgeRogues,
  walkFloor1ToServiceLift,
  expectFloor2,
} = require('./helpers');

// Actually plays the game: `/?floor=1&debug` drops us straight onto floor 1
// (no title / level-select), the frame probe reads the WebGL HUD text, and the
// run ends by riding the SERVICE LIFT to floor 2. See helpers.js for the
// `?floor=N&debug` + held-keys conventions.
test.describe('Open Miami - Level Completion', () => {
  test('game loads on floor 1 without errors and draws the HUD', async ({ page }) => {
    const errors = collectErrors(page);

    await loadFloor(page, 1);
    const texts = await lastFrameTexts(page);

    // The in-game HUD, not the title/level-select screen.
    expect(texts).toEqual(expect.arrayContaining(['Health:', 'Weapon:', 'Rogues:']));
    expect(Number(hudValue(texts, 'Health:'))).toBeGreaterThan(0);
    expect(Number(hudValue(texts, 'Rogues:'))).toBeGreaterThan(0);
    // Floor 1's objective names its exit.
    expect(texts.some((s) => s.includes('SERVICE LIFT'))).toBe(true);

    await page.screenshot({ path: 'test-results/01-floor1-loaded.png' });

    // WebGL is required: no error of any kind is tolerated.
    expect(errors).toEqual([]);
  });

  test('player moves, purges the rogues and takes the SERVICE LIFT to floor 2', async ({ page }) => {
    const errors = collectErrors(page);
    const canvas = await loadFloor(page, 1);

    // Debug overlay on (I), then K purges every rogue -> Rogues: 0 and the
    // all-dead scenario step opens the SERVICE LIFT. Done first so nothing
    // shoots at us while we walk.
    await purgeRogues(page);
    await page.screenshot({ path: 'test-results/02-rogues-purged.png' });

    // Move north a bit (entry lift is SE at (895,750)): the player's world
    // position changes and, as the camera follows the player, so does the frame.
    const posBefore = await playerPos(page);
    const before = await canvas.screenshot();
    await hold(page, 'w', 400);
    const after = await canvas.screenshot();
    const posAfter = await playerPos(page);
    expect(posBefore).not.toBeNull();
    expect(posAfter).not.toBeNull();
    expect(posAfter.y).toBeLessThan(posBefore.y - 10);
    expect(after.equals(before)).toBe(false);
    await page.screenshot({ path: 'test-results/03-moved.png' });
    let texts = await lastFrameTexts(page);
    expect(Number(hudValue(texts, 'Health:'))).toBeGreaterThan(0);
    expect(hudValue(texts, 'Rogues:')).toBe('0');

    // West along the bottom, then north into the open SERVICE LIFT (NW).
    await walkFloor1ToServiceLift(page);
    await page.waitForTimeout(1200); // dwell (0.6s) + the extraction card starts
    await page.screenshot({ path: 'test-results/04-extracting.png' });

    // The card ends -> floor 2 (COLD STORAGE, FREIGHT LIFT objective) loads.
    texts = await expectFloor2(page);
    expect(Number(hudValue(texts, 'Rogues:'))).toBeGreaterThan(0);
    await page.screenshot({ path: 'test-results/05-floor2-loaded.png' });

    expect(errors).toEqual([]);
  });
});

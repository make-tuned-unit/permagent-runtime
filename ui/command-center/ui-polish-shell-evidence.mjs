// Read-only local UI evidence. A blank startup frame is never a visual pass.
import { chromium } from 'playwright';

const browser = await chromium.launch({ channel: 'chrome', headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('http://127.0.0.1:5173/ui/');
  // Splash copy is content, but is not the interactive shell under review.
  let readyError;
  try {
    await page.locator('button[title="Collapse"], button[title="Expand"]').first().waitFor({ state: 'visible', timeout: 20000 });
  } catch (error) {
    readyError = error;
  }
  const text = await page.locator('body').innerText();
  await page.screenshot({ path: '/private/tmp/permagent-ui-shared-review.png' });
  console.log(JSON.stringify({ shellReady: !readyError, viewport: { width: 1280, height: 900 }, errors, text: text.slice(0, 4000) }));
  if (errors.length || readyError) process.exitCode = 1;
} finally {
  await browser.close();
}

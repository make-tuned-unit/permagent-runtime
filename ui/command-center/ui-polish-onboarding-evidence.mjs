// Synthetic first-run visual evidence: no real daemon, keys, downloads or models.
import { chromium } from 'playwright';

const browser = await chromium.launch({ channel: 'chrome', headless: true });
try {
  for (const theme of ['dark', 'silver']) {
    const page = await browser.newPage({ viewport: { width: 960, height: 720 } });
    const requests = [];
    await page.addInitScript(value => localStorage.setItem('permagent-theme', value), theme);
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.route('**/*', async route => {
      const url = new URL(route.request().url());
      if (url.origin === 'http://127.0.0.1:5173' &&
          (url.pathname.startsWith('/ui/') || url.pathname.startsWith('/@'))) {
        return route.continue();
      }
      requests.push({ path: url.pathname, method: route.request().method() });
      let value;
      if (url.pathname === '/config') value = { config: { wizard_complete: false } };
      if (url.pathname === '/config/providers') value = [];
      if (value !== undefined) return route.fulfill({ json: value });
      return route.fulfill({ status: 503, json: { error: 'isolated visual fixture: unavailable' } });
    });
    await page.goto('http://127.0.0.1:5173/ui/');
    await page.getByText('Welcome to Permagent', { exact: true }).waitFor({ state: 'visible' });
    const hiddenSteps = await page.locator('[inert][aria-hidden="true"]').count();
    const text = await page.locator('body').innerText();
    await page.screenshot({ path: `/private/tmp/permagent-onboarding-${theme}.png` });
    const forbidden = requests.filter(request => /ollama|dev-roots|voice|secret$/.test(request.path));
    console.log(JSON.stringify({ fixture: true, theme, hiddenSteps, requests, forbidden, text }));
    if (hiddenSteps !== 7 || forbidden.length) process.exitCode = 1;
    await page.close();
  }
} finally {
  await browser.close();
}

// THROWAWAY HARNESS (lane wq/wizard): real app + real api.ts in headless
// Chrome, with every daemon route answered by an in-memory fixture. No real
// daemon, keys, downloads or models. Deleted before the lane's final commit.
//
//   node wizard-evidence.mjs <baseUrl> <outDir>
import { chromium } from 'playwright';
import { mkdirSync, writeFileSync } from 'node:fs';

const BASE = process.argv[2] ?? 'http://127.0.0.1:5299/ui/';
const OUT = process.argv[3] ?? '/tmp/wizard-evidence';
mkdirSync(OUT, { recursive: true });
const sleep = ms => new Promise(r => setTimeout(r, ms));

function fixture(opts = {}) {
  const s = {
    provider: null, model: null, keys: new Set(), devRoots: undefined,
    ollamaUp: false, librarian: { enabled: false, start_time: '02:00', duration_minutes: 60, model: 'qwen3:8b', run_if_launched_in_window: false },
    checks: 0, identityPuts: 0, search: false, wizardComplete: false, log: [],
    ...opts,
  };
  const providers = () => [
    { name: 'acme', provider_type: 'Builtin', is_configured: s.keys.has('ACME_API_KEY'), is_default: s.provider === 'acme',
      metadata: { name: 'acme', display_name: 'Acme AI', description: '', default_model: 'acme-swift-2', known_models: [],
        config_keys: [{ name: 'ACME_API_KEY', required: true, secret: true }],
        setup_steps: ['Go to https://console.acme.example/keys', "Click 'Create key'", 'Copy the key and paste it above'] } },
    { name: 'nimbus', provider_type: 'Builtin', is_configured: s.keys.has('NIMBUS_API_KEY'), is_default: s.provider === 'nimbus',
      metadata: { name: 'nimbus', display_name: 'Nimbus Cloud', description: '', default_model: 'nimbus-large', known_models: [],
        config_keys: [{ name: 'NIMBUS_API_KEY', required: true, secret: true }, { name: 'NIMBUS_HOST', required: true, secret: false, default: 'https://api.nimbus.example' }] } },
    { name: 'orbit', provider_type: 'Builtin', is_configured: false, is_default: false,
      metadata: { name: 'orbit', display_name: 'Orbit (sign-in)', description: '', default_model: 'o', known_models: [],
        config_keys: [{ name: 'ORBIT_TOKEN', required: true, secret: true, oauth_flow: true }] } },
    { name: 'ollama', provider_type: 'Builtin', is_configured: true, is_default: s.provider === 'ollama',
      metadata: { name: 'ollama', display_name: 'Ollama', description: '', default_model: 'qwen3', known_models: [],
        config_keys: [{ name: 'OLLAMA_HOST', required: true, secret: false, default: 'localhost' }] } },
  ];
  const handle = async (route) => {
    const req = route.request();
    const url = new URL(req.url());
    const p = url.pathname;
    const m = req.method();
    const body = (() => { try { return JSON.parse(req.postData() || 'null'); } catch { return null; } })();
    s.log.push(`${m} ${p}`);
    const json = (value, status = 200) => route.fulfill({ status, json: value });
    if (p === '/config' && m === 'GET') {
      const config = { wizard_complete: s.wizardComplete };
      if (s.provider) { config.GOOSE_PROVIDER = s.provider; config.GOOSE_MODEL = s.model; }
      if (s.devRoots) config.dev_roots = s.devRoots;
      return json({ config });
    }
    if (p === '/config/providers') return json(providers());
    if (p === '/config/upsert') {
      if (body?.is_secret) s.keys.add(body.key);
      if (body?.key === 'dev_roots') s.devRoots = body.value;
      if (body?.key === 'wizard_complete') s.wizardComplete = body.value === true;
      if (body?.key === 'GOOSE_PROVIDER') s.provider = body.value;
      return json('ok');
    }
    if (p === '/config/set_provider') { s.provider = body.provider; s.model = body.model; return route.fulfill({ status: 200, body: '' }); }
    if (p === '/config/check_provider') {
      s.checks += 1;
      if (s.failFirstCheck && body.provider === 'acme' && !s.failedOnce) {
        s.failedOnce = true;
        await sleep(400);
        return json({ message: `Provider '${body.provider}' check failed: the provider rejected this key (Authentication failed. Status: 401 Unauthorized. Response: invalid api key)` }, 400);
      }
      await sleep(400);
      return json({ live: true, model: s.model, models_available: body.provider === 'ollama' ? 2 : 12, note: null });
    }
    if (p === '/config/read') return json(body?.key === 'TAVILY_API_KEY' && s.search ? { maskedValue: 'tvly-***' } : null);
    if (p.startsWith('/config/extensions')) {
      if (p.endsWith('/probe')) return json({ ok: true, toolCount: 2, tools: ['search'] });
      return json('ok');
    }
    if (p === '/api/agent/identity') {
      if (m === 'GET') return json({ first_name: 'Aria', last_name: null, nickname: null, display_name: 'Aria', traits: [], tone: '', opening_greeting: '', voice_id: null });
      s.identityPuts += 1;
      s.lastIdentity = body;
      if (s.failFirstIdentityPut && s.identityPuts === 1) return json({ message: "Couldn't save the agent's identity: disk is read-only" }, 500);
      return json({ ...body, display_name: body.first_name });
    }
    if (p === '/api/dev-roots') return json({ confirmed: s.devRoots ?? [], discovered: ['/Users/alex/code/atlas', '/Users/alex/code/notes-app'], home: '/Users/alex' });
    if (p === '/api/dev-roots/check') return json({ resolved: url.searchParams.get('path'), exists: true, has_repositories: true });
    if (p === '/api/librarian/schedule') {
      if (m === 'PUT') { s.librarian = body; return json(body); }
      return json(s.librarian);
    }
    if (p === '/api/ollama/status') {
      return json({ reachable: s.ollamaUp, installed: s.ollamaUp ? [
        { name: 'llama3.2:3b', size: 2019393189, digest: 'a', modified_at: '' },
        { name: 'qwen3:8b', size: 5225387923, digest: 'b', modified_at: '' },
      ] : [], running: [] });
    }
    if (p === '/api/ollama/start') { s.ollamaUp = true; return json({ launched: true, method: 'app' }); }
    if (p === '/api/ollama/pull') {
      await sleep(s.pullDelay ?? 3000);
      return route.fulfill({ status: 200, body: '{"status":"success"}\n' });
    }
    if (p === '/api/system_info') return json({ os: 'macos', os_version: '26', architecture: 'arm64', provider: null, model: null, enabled_extensions: [], total_ram_bytes: 16 * 1024 ** 3, cpu_brand: 'Apple M4', disk_free_bytes: 200e9 });
    return json({ message: 'fixture: not served' }, 503);
  };
  return { s, handle };
}

async function openWizard(browser, { width, height, theme, fx }) {
  const page = await browser.newPage({ viewport: { width, height } });
  await page.addInitScript(t => {
    localStorage.setItem('permagent-theme', t);
    localStorage.setItem('permagent-daemon-token', 'fixture-token');
  }, theme);
  await page.emulateMedia({ reducedMotion: 'reduce', colorScheme: theme === 'silver' ? 'light' : 'dark' });
  const origin = new URL(BASE).origin;
  await page.route('**/*', async route => {
    const url = new URL(route.request().url());
    if (url.origin === origin && (url.pathname.startsWith('/ui/') || url.pathname.startsWith('/@'))) return route.continue();
    if (url.origin !== origin) return route.abort();
    return fx.handle(route);
  });
  const errors = [];
  page.on('pageerror', e => errors.push(String(e)));
  await page.goto(BASE);
  await page.keyboard.press('Escape'); // skip the splash
  await page.getByRole('heading', { name: 'Welcome to Permagent' }).waitFor();
  return { page, errors };
}

/** Clipping audit: the step heading and primary button must be fully inside
 *  the viewport (after scrolling the body to its top) and nothing may overflow
 *  the page horizontally. */
async function audit(page, label) {
  return page.evaluate((label) => {
    const vw = window.innerWidth; const vh = window.innerHeight;
    const scroller = document.querySelector('main form > div');
    if (scroller) scroller.scrollTop = 0;
    const h1 = document.querySelector('main h1');
    const primary = document.querySelector('main form button[type="submit"]');
    const box = el => el ? el.getBoundingClientRect() : null;
    const hb = box(h1); const pb = box(primary);
    const inside = r => !!r && r.top >= 0 && r.left >= 0 && r.bottom <= vh && r.right <= vw;
    return {
      label,
      headingVisible: inside(hb),
      primaryVisible: inside(pb),
      horizontalOverflow: document.documentElement.scrollWidth > vw,
      progress: document.querySelector('[aria-live="polite"]')?.textContent ?? null,
    };
  }, label);
}

async function shot(page, name, ctx, audits) {
  await sleep(450);
  await page.screenshot({ path: `${OUT}/${ctx}-${name}.png` });
  audits.push(await audit(page, `${ctx}-${name}`));
}

const browser = await chromium.launch({ channel: 'chrome', headless: true });
const report = { runs: [] };
try {
  for (const [width, height] of (process.env.ONLY_KB ? [] : [[1024, 640], [1440, 900]])) {
    for (const theme of ['dark', 'silver']) {
      const ctx = `${width}x${height}-${theme === 'silver' ? 'light' : 'dark'}`;
      const audits = [];

      // ── Run A: full happy path, hosted key (rejected once, then working) ──
      const fx = fixture({ failFirstCheck: true, failFirstIdentityPut: true });
      const { page, errors } = await openWizard(browser, { width, height, theme, fx });
      await shot(page, '01-welcome', ctx, audits);
      await page.getByRole('button', { name: 'Get started' }).click();
      await page.getByRole('heading', { name: 'Choose a model' }).waitFor();
      await shot(page, '02-model', ctx, audits);

      await page.getByRole('radio', { name: /On this Mac/ }).check();
      await page.getByText("Ollama isn't running").waitFor();
      await shot(page, '03-model-local-not-running', ctx, audits);
      await page.getByRole('button', { name: 'Start Ollama' }).click();
      await page.getByRole('radiogroup', { name: 'Installed models' }).waitFor({ timeout: 8000 });
      await shot(page, '04-model-local-installed', ctx, audits);
      await page.getByRole('button', { name: 'Use this model' }).click();
      await page.getByText(/Ready · llama3\.2:3b/).waitFor();
      await shot(page, '05-model-local-ready', ctx, audits);

      await page.getByRole('radio', { name: /Decide later/ }).check();
      await shot(page, '06-model-later', ctx, audits);

      await page.getByRole('radio', { name: /Hosted provider/ }).check();
      await page.getByRole('button', { name: 'Provider', exact: true }).click();
      await shot(page, '07-model-hosted-providers', ctx, audits);
      await page.getByRole('option', { name: 'Acme AI' }).click();
      await page.getByRole('textbox', { name: 'API key' }).fill('acme-test-key-0000');
      await page.getByRole('button', { name: "Don't have a key?" }).click();
      await shot(page, '08-model-hosted-key', ctx, audits);
      await page.getByRole('button', { name: 'Test key' }).click();
      await page.getByText(/rejected this key/).waitFor();
      await shot(page, '09-model-hosted-rejected', ctx, audits);
      await page.getByRole('textbox', { name: 'API key' }).fill('acme-good-key-1111');
      await page.getByRole('button', { name: 'Test key' }).click();
      await page.getByText('Key works · acme-swift-2').waitFor();
      await shot(page, '10-model-hosted-works', ctx, audits);
      await page.getByRole('button', { name: 'Continue', exact: true }).click();

      await page.getByRole('heading', { name: 'Name your agent' }).waitFor();
      await shot(page, '11-agent', ctx, audits);
      await page.getByText('Direct', { exact: true }).click();
      await page.getByRole('button', { name: 'Continue', exact: true }).click();

      await page.getByRole('heading', { name: 'Where is your code?' }).waitFor();
      await page.getByText('/Users/alex/code/atlas').waitFor();
      await shot(page, '12-folders', ctx, audits);
      await page.getByRole('button', { name: /Use 2 folders/ }).click();

      await page.getByRole('heading', { name: 'Extras' }).waitFor();
      await shot(page, '13-extras', ctx, audits);
      await page.getByRole('button', { name: /Web search/ }).click();
      await page.getByRole('button', { name: 'Search service', exact: true }).click();
      await page.getByRole('option').first().click();
      await shot(page, '14-extras-search', ctx, audits);
      await page.getByRole('button', { name: /Web search/ }).click();
      await page.getByRole('button', { name: /Local memory model/ }).click();
      await page.getByText(/Suggested for 16 GB/).waitFor();
      await shot(page, '15-extras-memory', ctx, audits);
      await page.getByRole('button', { name: 'Turn on' }).click();
      await page.getByText('On · qwen3:8b').waitFor();
      await shot(page, '16-extras-memory-on', ctx, audits);
      await page.getByRole('button', { name: 'Continue', exact: true }).click();

      await page.getByRole('heading', { name: 'Ready when you are' }).waitFor();
      await page.getByText('Acme AI · acme-swift-2').waitFor();
      await page.getByLabel('First message (optional)').fill('Help me plan this week.');
      await shot(page, '17-review', ctx, audits);
      await page.getByRole('button', { name: 'Start Permagent' }).click();
      await page.getByText(/Couldn't save your setup/).waitFor();
      await shot(page, '18-review-save-failed', ctx, audits);
      await page.getByRole('button', { name: 'Retry' }).click();
      await page.getByTestId('wizard-shell').waitFor({ state: 'detached', timeout: 8000 });
      report.runs.push({ ctx, run: 'hosted', errors, identity: fx.s.lastIdentity, wizardComplete: fx.s.wizardComplete, devRoots: fx.s.devRoots, librarian: fx.s.librarian.enabled });
      await page.close();

      // ── Run B: no key, no local model → finish with "decide later" ──
      const fxB = fixture();
      const b = await openWizard(browser, { width, height, theme, fx: fxB });
      await b.page.getByRole('button', { name: 'Get started' }).click();
      await b.page.getByRole('radio', { name: /Decide later/ }).check();
      await b.page.getByRole('button', { name: 'Continue without a model' }).click();
      await b.page.getByRole('button', { name: 'Continue', exact: true }).click();
      await b.page.getByRole('button', { name: /I don't write code/ }).click();
      await b.page.getByRole('button', { name: 'Skip for now' }).click();
      await b.page.getByText(/can't reply yet/).waitFor();
      await shot(b.page, '19-review-no-model', ctx, audits);
      await b.page.getByRole('button', { name: 'Start Permagent' }).click();
      await b.page.getByTestId('wizard-shell').waitFor({ state: 'detached', timeout: 8000 });
      report.runs.push({ ctx, run: 'no-model', errors: b.errors, wizardComplete: fxB.s.wizardComplete, devRoots: fxB.s.devRoots, provider: fxB.s.provider });
      await b.page.close();

      // ── Run C: local model download in progress (Model step) ──
      const fxC = fixture({ ollamaUp: true, pullDelay: 60000 });
      fxC.s.ollamaUp = true;
      const c = await openWizard(browser, { width, height, theme, fx: fxC });
      // An Ollama with no models yet.
      await c.page.route('**/api/ollama/status', r => r.fulfill({ json: { reachable: true, installed: [], running: [] } }));
      await c.page.getByRole('button', { name: 'Get started' }).click();
      await c.page.getByRole('radio', { name: /On this Mac/ }).check();
      await c.page.getByRole('button', { name: 'Download' }).click();
      await c.page.getByRole('progressbar').waitFor();
      await shot(c.page, '20-model-local-downloading', ctx, audits);
      await c.page.getByRole('button', { name: 'Cancel' }).click();
      await c.page.getByText(/Download stopped/).waitFor();
      await shot(c.page, '21-model-local-download-cancelled', ctx, audits);
      report.runs.push({ ctx, run: 'download', errors: c.errors });
      await c.page.close();

      report.runs.push({ ctx, audits });
    }
  }

  // ── Run D: keyboard only, at the minimum window size ──
  const fxD = fixture();
  const d = await openWizard(browser, { width: 1024, height: 640, theme: 'dark', fx: fxD });
  const kb = d.page.keyboard;
  const steps = [];
  const where = async () => d.page.locator('[aria-live="polite"]').first().textContent();
  steps.push(await where());
  await kb.press('Enter'); // Get started has focus
  await d.page.getByRole('heading', { name: 'Choose a model' }).waitFor();
  steps.push(await where());
  steps.push({ focusOnEntry: await d.page.evaluate(() => document.activeElement?.getAttribute('value')) });
  await kb.press('Space'); // check "Hosted provider"
  await kb.press('Tab'); // provider select
  await kb.press('ArrowDown'); // opens, focuses first option
  await kb.press('Enter'); // choose it
  await kb.press('Tab'); // API key
  await kb.type('keyboard-key-123');
  await d.page.screenshot({ path: `${OUT}/kb-model-before-test.png` });
  await kb.press('Enter'); // test key (implicit submit)
  await d.page.getByText(/Key works/).waitFor();
  await d.page.screenshot({ path: `${OUT}/kb-model-after-test.png` });
  await kb.press('Enter'); // continue
  await d.page.getByRole('heading', { name: 'Name your agent' }).waitFor();
  steps.push(await where());
  steps.push({ focusOnEntry: await d.page.evaluate(() => document.activeElement?.tagName + ':' + (document.activeElement?.getAttribute('placeholder') ?? '')) });
  await kb.type('Nova');
  await kb.press('Escape'); // back to Model
  await d.page.getByRole('heading', { name: 'Choose a model' }).waitFor();
  steps.push({ afterEscape: await where() });
  await d.page.keyboard.press('Tab');
  // Return forward with the keyboard: Enter on the radio submits only when valid; use the footer.
  for (let i = 0; i < 12; i += 1) {
    const label = await d.page.evaluate(() => document.activeElement?.textContent ?? '');
    if (/^(Continue|Test saved key)$/.test(label.trim())) break;
    await kb.press('Tab');
  }
  steps.push({ footerFocused: await d.page.evaluate(() => document.activeElement?.textContent?.trim()) });
  await kb.press('Enter');
  await d.page.getByText(/Key works/).waitFor();
  await kb.press('Enter');
  await d.page.getByRole('heading', { name: 'Name your agent' }).waitFor();
  steps.push({ nameKept: await d.page.getByRole('textbox', { name: 'Name' }).inputValue() });
  await kb.press('Enter');
  await d.page.getByRole('heading', { name: 'Where is your code?' }).waitFor();
  steps.push(await where());
  for (let i = 0; i < 8; i += 1) {
    const label = await d.page.evaluate(() => document.activeElement?.textContent ?? '');
    if (/^Use 2 folders$/.test(label.trim())) break;
    await kb.press('Tab');
  }
  await kb.press('Enter');
  await d.page.getByRole('heading', { name: 'Extras' }).waitFor();
  steps.push(await where());
  await kb.press('Enter'); // focus is on the Web search disclosure: opens it
  steps.push({ searchExpanded: await d.page.getByRole('button', { name: /Web search/ }).getAttribute('aria-expanded') });
  for (let i = 0; i < 12; i += 1) {
    const label = await d.page.evaluate(() => document.activeElement?.textContent ?? '');
    if (/^Skip for now$/.test(label.trim())) break;
    await kb.press('Tab');
  }
  await kb.press('Enter');
  await d.page.getByRole('heading', { name: 'Ready when you are' }).waitFor();
  steps.push(await where());
  for (let i = 0; i < 14; i += 1) {
    const label = await d.page.evaluate(() => document.activeElement?.textContent ?? '');
    if (/^Start Permagent$/.test(label.trim())) break;
    await kb.press('Tab');
  }
  await kb.press('Enter');
  await d.page.getByTestId('wizard-shell').waitFor({ state: 'detached', timeout: 8000 });
  report.keyboard = { steps, errors: d.errors, identity: fxD.s.lastIdentity, wizardComplete: fxD.s.wizardComplete };
  await d.page.close();
} finally {
  await browser.close();
  writeFileSync(`${OUT}/evidence.json`, JSON.stringify(report, null, 2));
}
console.log(JSON.stringify(report, null, 1).slice(0, 6000));

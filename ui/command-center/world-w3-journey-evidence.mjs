// W3 actual pointer journeys. Coordinates are projected from the mounted R3F
// camera, then delivered as real mouse events; no callback or store setter is
// invoked by this harness.
import { chromium } from 'playwright';

const BASE = 'http://127.0.0.1:5173/ui/worldcensus.html?perf=1&dpr=1.5';
const browser = await chromium.launch({ channel: 'chrome', headless: true });

function dist(a, b) {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
}

function exactCenterText(text) {
  return [...document.querySelectorAll('div')].some((el) => {
    if (el.textContent?.trim() !== text) return false;
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0 && Math.abs(r.left + r.width / 2 - innerWidth / 2) < 140 &&
      Math.abs(r.top + r.height / 2 - innerHeight / 2) < 140;
  });
}

try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, deviceScaleFactor: 1 });
  await page.addInitScript(() => localStorage.setItem('permagent-theme', 'system'));
  await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'no-preference' });
  const failures = [];
  page.on('pageerror', error => failures.push(error.message));
  await page.goto(BASE);
  await page.waitForFunction(() =>
    window.__worldDebug?.projectWorldPoint &&
    window.__worldAgents?.getAgentPosition &&
    window.__worldDebug.assetStats?.()?.authoredAgents === 12,
    undefined,
    { timeout: 30000 },
  );

  // Establish user ownership of the orbit controls first. This is a real tiny
  // drag, which also disables idle auto-rotate so it cannot race the station
  // glide under test.
  const canvas = page.locator('canvas').first();
  const canvasBox = await canvas.boundingBox();
  if (!canvasBox) throw new Error('World canvas has no layout box');
  const orbitX = canvasBox.x + canvasBox.width / 2;
  const orbitY = canvasBox.y + canvasBox.height / 2;
  await page.mouse.move(orbitX, orbitY);
  await page.mouse.down();
  await page.mouse.move(orbitX + 12, orbitY + 4, { steps: 2 });
  await page.mouse.up();
  await page.waitForTimeout(120);

  const stations = [
    { id: 'library', point: [10, 1.4, 0], label: 'Build' },
    { id: 'observatory', point: [0, 1.4, 10], label: 'Brain' },
    { id: 'automate', point: [-10, 1.4, 0], label: 'Automate' },
    { id: 'workbench', point: [0, 1.4, -10], label: 'Lab · coming soon' },
  ];
  let stationHit = null;
  for (const station of stations) {
    const projected = await page.evaluate((point) => window.__worldDebug.projectWorldPoint(point), station.point);
    if (projected.ndcZ < -1 || projected.ndcZ > 1) continue;
    await page.mouse.move(projected.client[0], projected.client[1]);
    await page.waitForTimeout(180);
    const tooltipVisible = await page.evaluate(exactCenterText, station.label);
    if (tooltipVisible) {
      stationHit = { ...station, projected };
      break;
    }
  }
  if (!stationHit) throw new Error('No station interaction target produced its tooltip');

  const stationBefore = await page.evaluate(() => window.__worldDebug.cameraSnapshot());
  await page.evaluate(() => { delete window.__worldLastStationClick; });
  await page.mouse.click(stationHit.projected.client[0], stationHit.projected.client[1]);
  await page.waitForTimeout(900);
  const stationAfter = await page.evaluate(() => window.__worldDebug.cameraSnapshot());
  const stationCallback = await page.evaluate(() => window.__worldLastStationClick ?? null);
  if (dist(stationBefore.position, stationAfter.position) < 0.25) {
    throw new Error(JSON.stringify({
      reason: `Station ${stationHit.id} tooltip hit but camera did not glide`,
      stationHit,
      stationBefore,
      stationAfter,
      stationCallback,
    }));
  }

  // Put the camera in front of a live Henry pose and click
  // the projected character. This exercises AgentCharacterV2's real pointer
  // handler and WorldView's identity-to-HUD mapping.
  const agent = await page.evaluate(() => window.__worldAgents.getAgentPosition('henry'));
  if (!agent) throw new Error('Henry has no live motion position');
  await page.evaluate(({ agent }) => {
    window.__worldDebug.setCam(
      [agent.x, agent.y + 2.6, agent.z + 7],
      [agent.x, agent.y + 1.2, agent.z],
    );
  }, { agent });
  await page.waitForTimeout(350);
  // Keep this view through the pointer hit. Releasing it here lets the active
  // station glide/OrbitControls retarget before the click, making screen
  // coordinates stale. Release immediately after the actual hit so following
  // and Escape are still exercised with the production camera controller.
  await page.waitForFunction(() => window.__worldAgentScreens?.henry, undefined, { timeout: 5000 });
  const currentAgent = await page.evaluate(() => window.__worldAgents.getAgentPosition('henry'));
  if (!currentAgent) throw new Error('Henry disappeared from live motion position');
  const agentProjected = await page.evaluate(() => {
    const pose = window.__worldAgents.getAgentPosition('henry');
    return window.__worldDebug.projectWorldPoint([pose.x, pose.y + 1.2, pose.z]);
  });
  if (agentProjected.ndcZ < -1 || agentProjected.ndcZ > 1) throw new Error('Henry projected outside camera');
  await page.screenshot({ path: '/private/tmp/permagent-world-w3-agent-target.png' });
  await page.mouse.move(agentProjected.client[0], agentProjected.client[1]);
  await page.waitForTimeout(180);
  // The live rig can advance between its motion-store read and the next
  // rendered pose. Try the projected point first, then a small fixed set of
  // pixels around the visible Henry label in this fixed evidence viewport.
  const agentHeights = [1.2, 1.7, 0.8, 1.4, 2.0];
  let agentCallback = null;
  let agentClickPoint = agentProjected.client;
  let agentHover = null;
  for (const height of agentHeights) {
    // Leave the prior target so pointer-over is observed again, then project
    // the current pose rather than probing a stale pixel grid as Henry walks.
    await page.mouse.move(5, 5);
    await page.evaluate(() => {
      delete window.__worldLastAgentClick;
      delete window.__worldLastAgentHover;
    });
    const candidate = await page.evaluate((height) => {
      const pose = window.__worldAgents.getAgentPosition('henry');
      return window.__worldDebug.projectWorldPoint([pose.x, pose.y + height, pose.z]).client;
    }, height);
    await page.mouse.move(candidate[0], candidate[1]);
    await page.waitForTimeout(80);
    agentHover = await page.evaluate(() => window.__worldLastAgentHover ?? null);
    if (agentHover !== 'henry') continue;
    await page.mouse.click(candidate[0], candidate[1]);
    await page.waitForTimeout(220);
    agentCallback = await page.evaluate(() => window.__worldLastAgentClick ?? null);
    if (agentCallback === 'henry') {
      agentClickPoint = candidate;
      break;
    }
    if (agentCallback) {
      await page.keyboard.press('Escape');
      await page.waitForTimeout(120);
      await page.keyboard.press('Escape');
      await page.waitForTimeout(450);
    }
  }
  await page.evaluate(() => window.__worldDebug.clearCam());
  const closeButtonsAfterAgent = await page.locator('button[aria-label="Close"]').count();
  const agentBody = await page.locator('body').innerText();
  const walkingAfterAgent = agentBody.includes('WALKING');
  if (closeButtonsAfterAgent !== 1 || !walkingAfterAgent) {
    throw new Error(JSON.stringify({
      reason: 'Henry pointer hit did not open the mapped HUD and walking mode',
      agent: currentAgent,
      agentProjected,
      agentClickPoint,
      agentCallback,
      agentHover,
      closeButtonsAfterAgent,
      walkingAfterAgent,
      bodyTail: agentBody.slice(-600),
    }));
  }

  // A WALKING label alone is not movement evidence. Wait for the real camera
  // transition, hold the documented key, and check the live motion store.
  await page.waitForTimeout(1600);
  const walkBefore = await page.evaluate(() => window.__worldAgents.getAgentPosition('henry'));
  await page.keyboard.down('w');
  await page.waitForTimeout(250);
  await page.keyboard.up('w');
  const walkAfter = await page.evaluate(() => window.__worldAgents.getAgentPosition('henry'));
  const walked = Math.hypot(walkAfter.x - walkBefore.x, walkAfter.z - walkBefore.z);
  if (walked < 0.1) throw new Error('Walking mode did not move Henry in response to W');

  // Escape first closes the open HUD, then the existing camera escape affordance
  // returns from third-person to orbit. Both are real key events.
  await page.keyboard.press('Escape');
  await page.waitForTimeout(180);
  const closeButtonsAfterClose = await page.locator('button[aria-label="Close"]').count();
  if (closeButtonsAfterClose !== 0) throw new Error('Escape did not close the agent HUD');
  await page.keyboard.press('Escape');
  await page.waitForTimeout(1800);
  const orbitAfterEscape = await page.locator('body').innerText().then(text => text.includes('ORBIT'));
  if (!orbitAfterEscape) throw new Error('Second Escape did not return to orbit mode');

  if (failures.length) throw new Error(JSON.stringify({ failures }));
  console.log(JSON.stringify({
    station: {
      id: stationHit.id,
      label: stationHit.label,
      ndcZ: stationHit.projected.ndcZ,
      cameraMoved: dist(stationBefore.position, stationAfter.position),
    },
    agent: {
      id: 'henry',
      position: [currentAgent.x, currentAgent.y, currentAgent.z],
      clickPoint: agentClickPoint,
      hudOpened: closeButtonsAfterAgent === 1,
      walkingAfterAgent,
      walked,
      hudClosedByEscape: closeButtonsAfterClose === 0,
      orbitAfterEscape,
    },
    failures,
  }));
} finally {
  await browser.close();
}

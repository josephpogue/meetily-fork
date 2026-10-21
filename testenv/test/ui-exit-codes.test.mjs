// Pins ui.mjs's exit codes (0, 1, 3, 64) against a stub WebDriver server. Run: node --test "testenv/test/*.test.mjs"
// (a bare directory fails on node 25). The first run needs network: `npm ci` installs webdriverio from the lockfile.
// ui.mjs runs from testenv/build/ui-test (gitignored, outside the recipe hash) so no node_modules lands in testenv/ui.
import { test, before, after, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import net from 'node:net';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync, spawn } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const uiSrc = path.join(here, '..', 'ui');
const work = path.join(here, '..', 'build', 'ui-test');
const ELEMENT = 'element-6066-11e4-a52e-4f735466cecf';

let hoverRevealDecision;
let server, port, calls, bodies, xpaths, mode, sessionAttempts, pageClicks;
const SCRIPT_NOISE = 'self.__next_f.push([1,"1:HL[]"])';

before(async () => {
  fs.mkdirSync(work, { recursive: true });
  for (const f of ['ui.mjs', 'package.json', 'package-lock.json']) fs.copyFileSync(path.join(uiSrc, f), path.join(work, f));
  execFileSync('npm', ['ci', '--omit=dev', '--no-audit', '--no-fund', '--prefer-offline'], { cwd: work, stdio: 'inherit' });
  ({ hoverRevealDecision } = await import(pathToFileURL(path.join(work, 'ui.mjs')).href));
  server = http.createServer((req, res) => {
    calls.push(`${req.method} ${req.url}`);
    // The app dies after the session opens (session and window handle answered): reset every later request's connection.
    // The app answers the session and the window handle, then goes quiet (hang) or answers late (elementDelay ms).
    if (mode.hang && req.url.startsWith('/session/s1/') && req.url !== '/session/s1/window') return;
    if (mode.die === 'reset' && req.url !== '/session' && req.url !== '/session/s1/window') return void req.socket.destroy();
    const send = (status, value) => { res.writeHead(status, { 'content-type': 'application/json' }); res.end(JSON.stringify({ value })); };
    const err = (status, error, message) => send(status, { error, message, stacktrace: '' });
    let body = '';
    req.on('data', (d) => (body += d));
    req.on('end', () => {
      bodies.push(body);
      if (req.method === 'POST' && req.url === '/session') {
        sessionAttempts++;
        if (mode.session === 'no-window') return err(404, 'no such window', 'No window could be found'); // the plugin's own reply
        if (mode.session === 'hang') return; // never answered: the client's own timeout must end it
        if (mode.session === 'hang-first' && sessionAttempts === 1) return;
        if (mode.session === 'reset') return void req.socket.destroy();
        if (mode.session === 'broken') return err(500, 'session not created', 'stub says the app is unhappy');
        const created = () => send(200, { sessionId: 's1', capabilities: {} });
        return mode.sessionDelay ? void setTimeout(() => { if (!res.destroyed) created(); }, mode.sessionDelay) : created();
      }
      if (req.method === 'DELETE' && req.url === '/session/s1') return send(200, null);
      if (req.method === 'GET' && req.url === '/session/s1/window') return send(200, 'w1');
      // ui.mjs looks elements up with POST /elements (an empty list when nothing matches) and takes the first displayed one.
      // mode.find(xpath) returns the element ids that xpath would match; without it every xpath matches e1 (or nothing).
      if (req.method === 'POST' && req.url === '/session/s1/elements') {
        const xpath = JSON.parse(body).value;
        xpaths.push(xpath);
        const reply = () => send(200, (mode.find ? mode.find(xpath) : mode.elements ? ['e1'] : []).map((id) => ({ [ELEMENT]: id })));
        if (mode.elementHangFirst && xpaths.length === 1) return; // the first poll is never answered
        return mode.elementDelay ? void setTimeout(() => { if (!res.destroyed) reply(); }, mode.elementDelay) : reply();
      }
      if (req.method === 'POST' && req.url === '/session/s1/element') return mode.elements ? send(200, { [ELEMENT]: 'e1' }) : err(404, 'no such element', 'Unable to locate element');
      // webdriverio's isDisplayed runs a script instead of calling GET .../displayed.
      if (req.method === 'POST' && req.url === '/session/s1/execute/sync') {
        // The hover-reveal page script (decision, and with "hover-press" the in-page click): mode.reveal maps element id to its decision.
        if (body.includes('hover-reveal')) {
          const id = Object.keys(mode.reveal ?? {}).find((i) => JSON.stringify(JSON.parse(body).args).includes(`"${i}"`));
          if (mode.revealError) return err(500, 'javascript error', 'stub page script failed');
          if (body.includes('hover-press') && id && mode.reveal[id].ok) pageClicks.push(id);
          return send(200, (id && mode.reveal[id]) || { ok: false, why: 'stub' });
        }
        // webdriverio's isDisplayed passes the element reference as the script argument.
        if (!body.includes('innerText')) return send(200, !(mode.hiddenIds ?? []).some((id) => JSON.stringify(JSON.parse(body).args).includes(`"${id}"`)));
        if (mode.execute === 'rejected') return err(404, 'unknown command', 'execute is not allowed');
        return send(200, { text: mode.bigRead ? `${'x'.repeat(mode.bigRead)}\nend` : 'Welcome to Meetily\nGet Started', buttons: mode.buttons ?? [] });
      }
      if (req.method === 'GET' && req.url === '/session/s1/element/e1/text') return send(200, 'Welcome to Meetily' + SCRIPT_NOISE);
      if (req.method === 'POST' && /^\/session\/s1\/element\/e\d\/click$/.test(req.url)) return mode.clickRefused ? err(400, 'element not interactable', 'stub: not visible') : send(200, null);
      if (req.method === 'POST' && /^\/session\/s1\/element\/e\d\/clear$/.test(req.url)) return send(200, null);
      if (req.method === 'POST' && /^\/session\/s1\/element\/e\d\/value$/.test(req.url)) return send(200, null);
      send(404, { error: 'unknown command', message: req.url, stacktrace: '' });
    });
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  port = server.address().port;
});

after(() => { server?.closeAllConnections(); server?.close(); });
beforeEach(() => { calls = []; pageClicks = []; bodies = []; xpaths = []; sessionAttempts = 0; mode = { session: 'ok', elements: true, execute: 'ok' }; });

function ui(args, p = port, env = {}) {
  return new Promise((resolve) => {
    const started = Date.now();
    const child = spawn('node', ['ui.mjs', ...args], { cwd: work, env: { ...process.env, TESTENV_WEBDRIVER_PORT: String(p), TESTENV_UI_TIMEOUT_SECS: '2', ...env } });
    let stdout = '', stderr = '';
    child.stdout.on('data', (d) => (stdout += d));
    child.stderr.on('data', (d) => (stderr += d));
    child.on('close', (code) => resolve({ code, stdout, stderr, ms: Date.now() - started }));
  });
}
const deleted = () => calls.filter((c) => c === 'DELETE /session/s1').length;

test('exit 0: read prints the screen text and deletes the session', async () => {
  const r = await ui(['read']);
  assert.equal(r.code, 0);
  assert.match(r.stdout, /Welcome to Meetily/);
  assert.equal(deleted(), 1);
});

test('read prints the visible text from document.body.innerText, with no script text', async () => {
  const r = await ui(['read']);
  assert.equal(r.code, 0, r.stderr);
  assert.equal(r.stdout, 'Welcome to Meetily\nGet Started\n');
  assert.ok(!r.stdout.includes(SCRIPT_NOISE));
  assert.ok(calls.includes('POST /session/s1/execute/sync'));
  assert.ok(!calls.some((c) => c.endsWith('/element/e1/text')));
});

test('read falls back to getText when the add-on rejects execute', async () => {
  mode.execute = 'rejected';
  const r = await ui(['read']);
  assert.equal(r.code, 0, r.stderr);
  assert.match(r.stdout, /Welcome to Meetily/);
  assert.ok(calls.includes('POST /session/s1/element'), 'did not look up body for the fallback');
});

test('exit 0: wait finds the text', async () => {
  const r = await ui(['wait', 'Welcome', '5']);
  assert.equal(r.code, 0, r.stderr);
  assert.equal(deleted(), 1);
});

test('exit 0: click presses the element', async () => {
  const r = await ui(['click', 'Start']);
  assert.equal(r.code, 0, r.stderr);
  assert.ok(calls.includes('POST /session/s1/element/e1/click'));
});

test('exit 1: wait for text that never shows says "not found after Ns" and deletes the session', async () => {
  mode.elements = false;
  const r = await ui(['wait', 'Nope', '1']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /not found after 1s: Nope/);
  assert.equal(deleted(), 1);
});

test('exit 1: no such window during wait keeps retrying until the deadline, then says "not found after Ns"', async () => {
  mode.session = 'no-window';
  const r = await ui(['wait', 'Welcome', '3']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /not found after 3s: Welcome/);
  assert.doesNotMatch(r.stderr, /not answering/);
  assert.ok(r.ms >= 3000, `gave up after ${r.ms} ms, before the 3 s deadline`);
  assert.ok(calls.filter((c) => c === 'POST /session').length >= 2, 'did not retry POST /session');
});

test('exit 1: no such window on read prints the error and does not retry', async () => {
  mode.session = 'no-window';
  const r = await ui(['read']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /No window could be found/);
  assert.equal(calls.filter((c) => c === 'POST /session').length, 1);
});

test('exit 1: any other session error prints its message', async () => {
  mode.session = 'broken';
  const r = await ui(['wait', 'Welcome', '2']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /stub says the app is unhappy/);
});

test('exit 3: connection refused', async () => {
  const closed = await new Promise((resolve) => { const s = net.createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => resolve(p)); }); });
  const r = await ui(['wait', 'Welcome', '2'], closed);
  assert.equal(r.code, 3);
  assert.match(r.stderr, /not answering/);
});

test('exit 64: unknown verb', async () => {
  const r = await ui(['dance']);
  assert.equal(r.code, 64);
  assert.match(r.stderr, /usage/);
  assert.deepEqual(calls, []);
});

test('exit 64: a missing or empty argument is a usage mistake, and nothing is sent to the app', async () => {
  for (const args of [['click'], ['click', '  '], ['type', 'Ask a question'], ['type'], ['wait'], ['wait', 'Welcome', 'soon']]) {
    const r = await ui(args);
    assert.equal(r.code, 64, `${args.join(' ')}: ${r.stderr}`);
    assert.match(r.stderr, new RegExp(`usage: ui.mjs ${args[0]} `));
  }
  assert.deepEqual(calls, []);
});

test('read prints more than 64 KB, so the exit does not cut piped output', async () => {
  mode.bigRead = 300000;
  const r = await ui(['read']);
  assert.equal(r.code, 0, r.stderr);
  assert.equal(r.stdout.length, 300000 + '\nend\n'.length);
  assert.ok(r.stdout.endsWith('\nend\n'));
});

// Matching: visible text OR aria-label OR placeholder, exact first, then contains.
test('click matches an icon-only button by its aria-label: the exact xpath names @aria-label, lowercased', async () => {
  const r = await ui(['click', 'Start recording']);
  assert.equal(r.code, 0, r.stderr);
  assert.ok(xpaths[0].includes('@aria-label') && xpaths[0].includes('="start recording"'), xpaths[0]);
  assert.ok(calls.includes('POST /session/s1/element/e1/click'));
});

test('click matches trimmed and ignoring case: the target is trimmed and lowercased, the page side translated', async () => {
  const r = await ui(['click', '  Start   Recording ']);
  assert.equal(r.code, 0, r.stderr);
  assert.ok(xpaths[0].includes('="start recording"'), xpaths[0]);
  assert.ok(xpaths[0].includes("translate(normalize-space(@aria-label), 'ABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwxyz')"), xpaths[0]);
});

test('exit 1: click never falls back to contains, so "Start" does not press "Get Started"', async () => {
  mode.find = (xp) => (xp.includes('contains(') ? ['e2'] : []);
  const r = await ui(['click', 'Start']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /nothing to click: Start/);
  assert.ok(!calls.some((c) => c.endsWith('/click')), 'clicked something');
  assert.ok(xpaths.length > 0 && xpaths.every((x) => !x.includes('contains(')), 'click asked for a contains match');
});

test('exit 1: type never falls back to contains either', async () => {
  mode.find = (xp) => (xp.includes('contains(') ? ['e2'] : []);
  const r = await ui(['type', 'Meeting', 'x']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /no field: Meeting/);
  assert.ok(xpaths.length > 0 && xpaths.every((x) => !x.includes('contains(')), 'type asked for a contains match');
});

test('wait: exact match wins over contains, and contains is not asked when the exact one is there', async () => {
  mode.find = (xp) => (xp.includes('contains(') ? ['e2'] : ['e1']);
  const r = await ui(['wait', 'Stop recording', '5']);
  assert.equal(r.code, 0, r.stderr);
  assert.equal(xpaths.length, 1, 'looked up contains although the exact match was there');
});

test('wait keeps contains as the fallback: nothing matches exactly, a longer label does', async () => {
  mode.find = (xp) => (xp.includes('contains(') ? ['e2'] : []);
  const r = await ui(['wait', 'Start rec', '5']);
  assert.equal(r.code, 0, r.stderr);
  assert.equal(xpaths.length, 2);
  assert.ok(xpaths[1].includes('contains(@aria-label, "Start rec")'), xpaths[1]);
});

test('wait, click and type all match text, aria-label and placeholder', async () => {
  for (const args of [['wait', 'Stop recording', '5'], ['click', 'Stop recording'], ['type', 'Meeting title', 'x']]) {
    const r = await ui(args);
    assert.equal(r.code, 0, r.stderr);
  }
  assert.ok(xpaths.every((x) => x.includes('@aria-label') && x.includes('@placeholder') && x.includes('text()')));
});

test('the first displayed match is used: a hidden twin is skipped', async () => {
  mode.find = () => ['e2', 'e1'];
  mode.hiddenIds = ['e2'];
  const r = await ui(['click', 'Start recording']);
  assert.equal(r.code, 0, r.stderr);
  assert.ok(calls.includes('POST /session/s1/element/e1/click'));
});

test('exit 1: click on a label nothing carries says "nothing to click"', async () => {
  mode.elements = false;
  const r = await ui(['click', 'Start recording']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /nothing to click: Start recording/);
  assert.equal(xpaths.length > 2, true, 'did not poll');
});

// read: visible text, then the aria-labels of icon-only buttons.
test('read lists icon-only buttons after the text, one per line, prefixed "button: "', async () => {
  mode.buttons = ['Start recording', 'Home'];
  const r = await ui(['read']);
  assert.equal(r.code, 0, r.stderr);
  assert.equal(r.stdout, 'Welcome to Meetily\nGet Started\nbutton: Start recording\nbutton: Home\n');
});

test('the read script asks only for visible buttons with an aria-label and no text of their own', async () => {
  await ui(['read']);
  const i = calls.indexOf('POST /session/s1/execute/sync');
  const script = JSON.parse(bodies[i]).script;
  assert.match(script, /button\[aria-label\]/);
  assert.match(script, /getClientRects/);
  assert.match(script, /innerText\.trim\(\)/);
});

test('exit 0: type clears the field and sends the text', async () => {
  const r = await ui(['type', 'Meeting title', 'Quarterly review']);
  assert.equal(r.code, 0, r.stderr);
  assert.ok(calls.includes('POST /session/s1/element/e1/clear'));
  const i = calls.indexOf('POST /session/s1/element/e1/value');
  assert.ok(i >= 0, 'no value request');
  assert.match(bodies[i], /Quarterly review/);
  assert.equal(deleted(), 1);
});

test('exit 1: type into a field that is not there says "no field"', async () => {
  mode.elements = false;
  const r = await ui(['type', 'Nope', 'text']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /no field: Nope/);
});

test('exit 3 at once: the app drops the connection after the session opens (wait)', async () => {
  mode.die = 'reset';
  const r = await ui(['wait', 'Never shows', '20']);
  assert.equal(r.code, 3, r.stderr);
  assert.match(r.stderr, /not answering/);
  assert.ok(r.ms < 5000, `took ${r.ms} ms, so it polled to the deadline`);
});

test('exit 3 at once: the app drops the connection after the session opens (click)', async () => {
  mode.die = 'reset';
  const r = await ui(['click', 'Start']);
  assert.equal(r.code, 3, r.stderr);
  assert.ok(r.ms < 5000, `took ${r.ms} ms`);
});

test('exit 3 at once: the app stops listening after the session opens', async () => {
  const dying = http.createServer((req, res) => {
    req.resume();
    req.on('end', () => {
      const send = (v) => { res.writeHead(200, { 'content-type': 'application/json', connection: 'close' }); res.end(JSON.stringify({ value: v })); };
      if (req.method === 'POST' && req.url === '/session') { send({ sessionId: 's1', capabilities: {} }); setTimeout(() => { dying.close(); dying.closeAllConnections(); }, 400); return; }
      if (req.method === 'GET' && req.url === '/session/s1/window') return send('w1');
      return send(null);
    });
  });
  await new Promise((r) => dying.listen(0, '127.0.0.1', r));
  const r = await ui(['wait', 'Never shows', '20'], dying.address().port);
  assert.equal(r.code, 3, r.stderr);
  assert.ok(r.ms < 6000, `took ${r.ms} ms`);
});

// A slow or hung app that does answer late is a timeout (exit 1), never "nothing is answering" (exit 3).
test('exit 1, not 3: a slow but alive app answers "no such element" after 3 s and wait Missing 2 ends as not found', async () => {
  mode.elementDelay = 3000;
  mode.elements = false;
  const r = await ui(['wait', 'Missing', '2'], port, { TESTENV_UI_TIMEOUT_SECS: '5' });
  assert.equal(r.code, 1, r.stderr);
  assert.match(r.stderr, /not found after 2s: Missing/);
  assert.doesNotMatch(r.stderr, /not answering|did not answer/);
  assert.ok(r.ms < 8000, `took ${r.ms} ms`);
});

for (const [verb, args] of [['wait', ['wait', 'Missing', '3']], ['click', ['click', 'Start']], ['type', ['type', 'Field', 'x']], ['read', ['read']]]) {
  test(`exit 1: a hung app makes ${verb} say "app did not answer within 2s" and not exit 3`, async () => {
    mode.hang = true;
    const r = await ui(args);
    assert.equal(r.code, 1, r.stderr);
    assert.match(r.stderr, /app did not answer within 2s/);
    assert.doesNotMatch(r.stderr, /not answering/);
    assert.ok(r.ms < 9000, `took ${r.ms} ms`);
  });
}

test('exit 0: wait keeps polling after one poll that never answers, and finds the text', async () => {
  mode.elementHangFirst = true;
  const r = await ui(['wait', 'Welcome', '30']);
  assert.equal(r.code, 0, r.stderr);
  assert.ok(xpaths.length >= 2, `only ${xpaths.length} poll(s)`);
  assert.ok(r.ms < 10000, `took ${r.ms} ms`);
});

test('exit 1: wait with every poll unanswered ends at the deadline, not at the first slow poll', async () => {
  mode.hang = true;
  const r = await ui(['wait', 'Missing', '5']);
  assert.equal(r.code, 1, r.stderr);
  assert.match(r.stderr, /app did not answer within 2s/);
  assert.ok(r.ms >= 5000, `ended at ${r.ms} ms, before the 5 s deadline`);
  assert.ok(r.ms < 12000, `took ${r.ms} ms`);
});

test('exit 0: read waits out an 11 s POST /session hold with the default session timeout', async () => {
  mode.sessionDelay = 11000;
  const r = await ui(['read'], port, { TESTENV_UI_TIMEOUT_SECS: undefined });
  assert.equal(r.code, 0, r.stderr);
  assert.match(r.stdout, /Welcome to Meetily/);
  assert.ok(r.ms >= 10500, `answered in ${r.ms} ms`);
});

test('exit 1: POST /session that never answers ends wait after one request timeout, not the whole wait', async () => {
  mode.session = 'hang';
  const r = await ui(['wait', 'Welcome', '1']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /app did not answer within 2s/);
  assert.ok(r.ms < 6000, `took ${r.ms} ms`);
});

test('exit 0: wait retries a POST /session that timed out and succeeds on the next attempt', async () => {
  mode.session = 'hang-first';
  const r = await ui(['wait', 'Welcome', '20']);
  assert.equal(r.code, 0, r.stderr);
  assert.ok(sessionAttempts >= 2, `only ${sessionAttempts} POST /session`);
});

test('exit 3: the app resets the connection on POST /session itself', async () => {
  mode.session = 'reset';
  const r = await ui(['wait', 'Welcome', '20']);
  assert.equal(r.code, 3, r.stderr);
  assert.ok(r.ms < 5000, `took ${r.ms} ms`);
});

// e2 and e4 are dismiss X buttons that are opacity-0 (not displayed) until their row is hovered; e1 is another text on screen.
const OK = { ok: true, why: 'hover-revealed' };
const NO = { ok: false, why: 'stub' };
const hiddenDismiss = (reveal, ids = ['e2']) => { mode.find = (x) => (x.includes('dismiss') ? ids : []); mode.hiddenIds = ids; mode.reveal = reveal; };
const clicked = () => calls.filter((c) => /\/element\/e\d\/click$/.test(c));
const DISMISS = 'Dismiss to-do: Call the zoo';

test('hover is no longer a verb: usage, exit 64', async () => {
  const r = await ui(['hover', 'Call the zoo']);
  assert.equal(r.code, 64);
  assert.match(r.stderr, /usage: ui\.mjs wait\|click\|type\|read/);
});

test('exit 0: click on a displayed element is the plain path, no reveal script runs', async () => {
  const r = await ui(['click', 'Call the zoo']);
  assert.equal(r.code, 0, r.stderr);
  assert.deepEqual(clicked(), ['POST /session/s1/element/e1/click']);
  assert.ok(!bodies.some((b) => b.includes('hover-reveal')));
  assert.ok(!r.stdout.includes('hover-revealed'));
});

test('exit 0: click on a hover-revealed control presses it and says so', async () => {
  hiddenDismiss({ e2: OK });
  const r = await ui(['click', DISMISS]);
  assert.equal(r.code, 0, r.stderr);
  assert.match(r.stdout, /pressed as a hover-revealed control: Dismiss to-do: Call the zoo/);
  assert.deepEqual(clicked(), ['POST /session/s1/element/e2/click']);
  assert.ok(!calls.some((c) => c.endsWith('/actions')), 'no pointer move is used');
});

test('exit 0: click tries every exact match, not only the first', async () => {
  hiddenDismiss({ e2: NO, e4: OK }, ['e2', 'e4']);
  const r = await ui(['click', DISMISS]);
  assert.equal(r.code, 0, r.stderr);
  assert.deepEqual(clicked(), ['POST /session/s1/element/e4/click']);
});

test('exit 0: when the plugin refuses the click, the control is pressed inside the page', async () => {
  hiddenDismiss({ e2: OK });
  mode.clickRefused = true;
  const r = await ui(['click', DISMISS]);
  assert.equal(r.code, 0, r.stderr);
  assert.deepEqual(pageClicks, ['e2']);
  assert.match(r.stdout, /pressed as a hover-revealed control/);
});

test('exit 1: click on a control the decision refuses never presses it', async () => {
  hiddenDismiss({ e2: NO });
  const r = await ui(['click', DISMISS], port, { TESTENV_UI_TIMEOUT_SECS: '2' });
  assert.equal(r.code, 1);
  assert.match(r.stderr, /nothing to click: Dismiss to-do: Call the zoo/);
  assert.deepEqual(clicked(), []);
  assert.deepEqual(pageClicks, []);
  assert.ok(r.ms >= 9000 && r.ms < 13000, `took ${r.ms} ms, the usual ~10 s`);
});

test('exit 1: an error in the reveal script does not skip the rest of the wait, and a control drawn late is still pressed', async () => {
  hiddenDismiss({ e2: OK });
  mode.revealError = true;
  setTimeout(() => { mode.revealError = false; }, 2500);
  const r = await ui(['click', DISMISS]);
  assert.equal(r.code, 0, r.stderr);
  assert.ok(r.ms >= 2500, `gave up early at ${r.ms} ms`);
  assert.deepEqual(clicked(), ['POST /session/s1/element/e2/click']);
});

// The decision itself, fed fake computed values. A dismiss X inside a li.group row, hidden by opacity-0 alone.
const visible = (over = {}) => ({ display: 'block', visibility: 'visible', opacity: '1', isRow: false, ...over });
const facts = (over = {}) => ({
  chain: [visible({ opacity: '0' }), visible({ isRow: true }), visible(), visible()],
  rect: { left: 300, top: 100, right: 316, bottom: 116, width: 16, height: 16 },
  viewport: { width: 1280, height: 800 },
  tag: 'button', hasHref: false, disabled: false, ariaDisabled: null, hitSelf: true,
  ...over,
});

test('decision: an opacity-0 button in a visible row, in view and on top, is pressable', () => {
  assert.equal(hoverRevealDecision(facts()).ok, true);
});

test('decision: a link with an href is pressable, an a without one is not', () => {
  assert.equal(hoverRevealDecision(facts({ tag: 'a', hasHref: true })).ok, true);
  assert.equal(hoverRevealDecision(facts({ tag: 'a', hasHref: false })).ok, false);
});

test('decision: opacity 0 on a wrapper between the element and its row also counts', () => {
  const chain = [visible(), visible({ opacity: '0' }), visible({ isRow: true }), visible()];
  assert.equal(hoverRevealDecision(facts({ chain })).ok, true);
});

const refused = {
  'not hidden by opacity (hidden some other way)': facts({ chain: [visible(), visible({ isRow: true }), visible()] }),
  'display none on the element': facts({ chain: [visible({ opacity: '0', display: 'none' }), visible({ isRow: true })] }),
  'display none on an ancestor': facts({ chain: [visible({ opacity: '0' }), visible({ isRow: true, display: 'none' })] }),
  'visibility hidden on the element': facts({ chain: [visible({ opacity: '0', visibility: 'hidden' }), visible({ isRow: true })] }),
  'visibility collapse on an ancestor': facts({ chain: [visible({ opacity: '0' }), visible({ isRow: true }), visible({ visibility: 'collapse' })] }),
  'the row itself is opacity 0': facts({ chain: [visible({ opacity: '0' }), visible({ isRow: true, opacity: '0' })] }),
  'an ancestor above the row is opacity 0': facts({ chain: [visible({ opacity: '0' }), visible({ isRow: true }), visible({ opacity: '0' })] }),
  'no row (no group ancestor)': facts({ chain: [visible({ opacity: '0' }), visible(), visible()] }),
  'zero-size box': facts({ rect: { left: 300, top: 100, right: 300, bottom: 100, width: 0, height: 0 } }),
  'box off the right edge': facts({ rect: { left: 1270, top: 100, right: 1286, bottom: 116, width: 16, height: 16 } }),
  'box below the viewport': facts({ rect: { left: 300, top: 790, right: 316, bottom: 806, width: 16, height: 16 } }),
  'box above the viewport': facts({ rect: { left: 300, top: -4, right: 316, bottom: 12, width: 16, height: 16 } }),
  'a div': facts({ tag: 'div' }),
  'a disabled button': facts({ disabled: true }),
  'aria-disabled button': facts({ ariaDisabled: 'true' }),
  'covered by another element': facts({ hitSelf: false }),
  'no facts': undefined,
  'empty chain': facts({ chain: [] }),
};
for (const [name, f] of Object.entries(refused)) {
  test(`decision: refused when ${name}`, () => {
    assert.equal(hoverRevealDecision(f).ok, false);
  });
}

test('decision: aria-disabled="false" does not disable', () => {
  assert.equal(hoverRevealDecision(facts({ ariaDisabled: 'false' })).ok, true);
});

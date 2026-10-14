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
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const uiSrc = path.join(here, '..', 'ui');
const work = path.join(here, '..', 'build', 'ui-test');
const ELEMENT = 'element-6066-11e4-a52e-4f735466cecf';

let server, port, calls, bodies, mode;
const SCRIPT_NOISE = 'self.__next_f.push([1,"1:HL[]"])';

before(async () => {
  fs.mkdirSync(work, { recursive: true });
  for (const f of ['ui.mjs', 'package.json', 'package-lock.json']) fs.copyFileSync(path.join(uiSrc, f), path.join(work, f));
  execFileSync('npm', ['ci', '--omit=dev', '--no-audit', '--no-fund', '--prefer-offline'], { cwd: work, stdio: 'inherit' });
  server = http.createServer((req, res) => {
    calls.push(`${req.method} ${req.url}`);
    // The app dies after the session opens (session and window handle answered): reset every later request's connection.
    if (mode.die === 'reset' && req.url !== '/session' && req.url !== '/session/s1/window') return void req.socket.destroy();
    const send = (status, value) => { res.writeHead(status, { 'content-type': 'application/json' }); res.end(JSON.stringify({ value })); };
    const err = (status, error, message) => send(status, { error, message, stacktrace: '' });
    let body = '';
    req.on('data', (d) => (body += d));
    req.on('end', () => {
      bodies.push(body);
      if (req.method === 'POST' && req.url === '/session') {
        if (mode.session === 'no-window') return err(404, 'no such window', 'No window could be found'); // the plugin's own reply
        // The plugin holds POST /session up to 10 s before it says so; an aborted client leaves the reply unsent.
        if (mode.session === 'no-window-slow') return void setTimeout(() => { if (!res.destroyed) err(404, 'no such window', 'No window could be found'); }, 6000);
        if (mode.session === 'broken') return err(500, 'session not created', 'stub says the app is unhappy');
        return send(200, { sessionId: 's1', capabilities: {} });
      }
      if (req.method === 'DELETE' && req.url === '/session/s1') return send(200, null);
      if (req.method === 'GET' && req.url === '/session/s1/window') return send(200, 'w1');
      if (req.method === 'POST' && req.url === '/session/s1/element') return mode.elements ? send(200, { [ELEMENT]: 'e1' }) : err(404, 'no such element', 'Unable to locate element');
      // webdriverio's isDisplayed runs a script instead of calling GET .../displayed.
      if (req.method === 'POST' && req.url === '/session/s1/execute/sync') {
        if (!body.includes('innerText')) return send(200, true);
        if (mode.execute === 'rejected') return err(404, 'unknown command', 'execute is not allowed');
        return send(200, mode.bigRead ? `${'x'.repeat(mode.bigRead)}\nend` : 'Welcome to Meetily\nGet Started');
      }
      if (req.method === 'GET' && req.url === '/session/s1/element/e1/text') return send(200, 'Welcome to Meetily' + SCRIPT_NOISE);
      if (req.method === 'POST' && req.url === '/session/s1/element/e1/click') return send(200, null);
      if (req.method === 'POST' && req.url === '/session/s1/element/e1/clear') return send(200, null);
      if (req.method === 'POST' && req.url === '/session/s1/element/e1/value') return send(200, null);
      send(404, { error: 'unknown command', message: req.url, stacktrace: '' });
    });
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  port = server.address().port;
});

after(() => { server?.closeAllConnections(); server?.close(); });
beforeEach(() => { calls = []; bodies = []; mode = { session: 'ok', elements: true, execute: 'ok' }; });

function ui(args, p = port) {
  return new Promise((resolve) => {
    const started = Date.now();
    const child = spawn('node', ['ui.mjs', ...args], { cwd: work, env: { ...process.env, TESTENV_WEBDRIVER_PORT: String(p) } });
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

test('read prints more than 64 KB, so the exit does not cut piped output', async () => {
  mode.bigRead = 300000;
  const r = await ui(['read']);
  assert.equal(r.code, 0, r.stderr);
  assert.equal(r.stdout.length, 300000 + '\nend\n'.length);
  assert.ok(r.stdout.endsWith('\nend\n'));
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

test('exit 1: a slow no-window reply does not push wait past its deadline', async () => {
  mode.session = 'no-window-slow';
  const r = await ui(['wait', 'Welcome', '2']);
  assert.equal(r.code, 1);
  assert.match(r.stderr, /not found after 2s: Welcome/);
  assert.ok(r.ms < 4000, `took ${r.ms} ms for a 2 s wait`);
});

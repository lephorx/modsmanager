import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
import assert from 'node:assert/strict';

const target = 'https://www.nexusmods.com/bonelab/mods/12?file_id=34&nmm=1';
const script = readFileSync(new URL('../src-tauri/src/nexus_helper.js', import.meta.url), 'utf8').replace('__MM_INFO__', JSON.stringify({ url: target }));
function run({ login = false, disabled = false, text = 'Slow download', offFile = false, blocked = false, consent = false, loggedOut = false, modernPath = false, signedIn = false } = {}) {
  let clicks = 0;
  let tick;
  let timeout;
  const navigations = [];
  const state = { blocked, consent, loggedOut, signedIn };
  const button = { textContent: text, disabled, getAttribute: () => null, getClientRects: () => [1], scrollIntoView() {}, getBoundingClientRect: () => ({ left: 0, top: 0, width: 100, height: 40 }), contains: () => false, click: () => clicks++ };
  const location = {
    hostname: login ? 'users.nexusmods.com' : 'www.nexusmods.com',
    pathname: offFile ? '/' : modernPath ? '/games/bonelab/mods/12' : '/bonelab/mods/12', search: '?file_id=34',
    set href(value) { navigations.push(value); }, replace: (value) => navigations.push(value),
  };
  vm.runInNewContext(script, {
    URL, URLSearchParams, location, window: {}, getComputedStyle: () => ({ visibility: "visible" }),
    document: {
      querySelector: (selector) => selector.includes("sign_out") && state.signedIn ? button : null,
      querySelectorAll: (selector) => selector === "button, a"
        ? [button, ...(state.loggedOut ? [{ ...button, textContent: "Log in" }] : [])]
        : state.consent ? [{ ...button, tagName: "IFRAME" }] : [],
      elementFromPoint: () => state.blocked ? {} : button,
    },
    setInterval: (fn) => { tick = fn; }, setTimeout: (fn) => { timeout = fn; },
    clearInterval() {}, clearTimeout() {},
  });
  return { state, button, navigations, tick: () => tick(), timeout: () => timeout(), clicks: () => clicks };
}
test('clicks Slow download once, even after repeated checks', () => {
  const page = run();
  page.tick(); page.tick();
  assert.equal(page.clicks(), 1);
  assert.deepEqual(page.navigations, []);
});
test('waits until Slow download is enabled', () => {
  const page = run({ disabled: true });
  assert.equal(page.clicks(), 0);
  page.button.disabled = false; page.tick();
  assert.equal(page.clicks(), 1);
});
test('never selects a premium download button', () => {
  const page = run({ text: 'Fast download' });
  page.tick();
  assert.equal(page.clicks(), 0);
  page.timeout();
  assert.deepEqual(page.navigations, ['mm-nexus://show']);
});
test('reveals login once without clicking', () => {
  const page = run({ login: true });
  page.tick(); page.timeout();
  assert.equal(page.clicks(), 0);
  assert.deepEqual(page.navigations, ['mm-nexus://show', 'mm-nexus://signed-out']);
});
test('does not infer login from a homepage redirect', () => {
  const page = run({ offFile: true });
  assert.equal(page.clicks(), 0);
  assert.deepEqual(page.navigations, []);
});

test('waits for cookie consent and resumes without consuming the download click', () => {
  const page = run({ consent: true, blocked: true });
  assert.equal(page.clicks(), 0);
  assert.deepEqual(page.navigations, ['mm-nexus://show']);
  page.state.consent = false;
  page.tick();
  assert.equal(page.clicks(), 0);
  page.state.blocked = false;
  page.tick(); page.tick();
  assert.equal(page.clicks(), 1);
});
test('shows the logged-out file page and resumes after login', () => {
  const page = run({ loggedOut: true });
  assert.equal(page.clicks(), 0);
  assert.deepEqual(page.navigations, ['mm-nexus://signed-out', 'mm-nexus://show']);
  page.state.loggedOut = false; page.tick();
  assert.equal(page.clicks(), 1);
});
test('accepts the modern Nexus route without redirecting in a loop', () => {
  const page = run({ modernPath: true });
  assert.equal(page.clicks(), 1);
  assert.deepEqual(page.navigations, []);
});

test('reports a confirmed login once so the backend can save it and resume the file', () => {
  const page = run({ offFile: true, signedIn: true });
  page.tick(); page.tick();
  assert.equal(page.clicks(), 0);
  assert.deepEqual(page.navigations, ['mm-nexus://signed-in']);
});

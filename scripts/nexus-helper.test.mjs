import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
import assert from 'node:assert/strict';

const target = 'https://www.nexusmods.com/bonelab/mods/12?file_id=34&nmm=1';
const source = readFileSync(new URL('../src-tauri/src/nexus_helper.js', import.meta.url), 'utf8');
function run({ login = false, disabled = false, text = 'Slow download', offFile = false, blocked = false, consent = false, loggedOut = false, modernPath = false, signedIn = false, loginReturn = false, header = true, challenge = false, shadow = false } = {}) {
  let clicks = 0;
  let tick;
  let timeout;
  const navigations = [];
  const messages = [];
  let now = 0;
  const state = { blocked, consent, loggedOut, signedIn, header, challenge };
  const button = { textContent: text, disabled, getAttribute: () => null, getClientRects: () => [1], scrollIntoView() {}, getBoundingClientRect: () => ({ left: 0, top: 0, width: 100, height: 40 }), contains: () => false, click: () => clicks++ };
  // Current Nexus pages render the button inside <mod-file-download>'s shadow root.
  const host = { tagName: 'MOD-FILE-DOWNLOAD' };
  if (shadow) {
    host.shadowRoot = {
      querySelectorAll: (selector) => selector === "button, a" ? [button] : [],
      elementFromPoint: () => state.blocked ? {} : button,
    };
    button.getRootNode = () => host.shadowRoot;
  }
  const location = {
    hostname: login ? 'users.nexusmods.com' : 'www.nexusmods.com',
    pathname: offFile ? '/' : modernPath ? '/games/bonelab/mods/12' : '/bonelab/mods/12', search: '?file_id=34',
    set href(value) { navigations.push(value); }, replace: (value) => navigations.push(value),
  };
  vm.runInNewContext(source.replace('__MM_INFO__', JSON.stringify({ url: target, loginReturn })), {
    URL, URLSearchParams, Date: { now: () => now }, location, window: {}, getComputedStyle: () => ({ visibility: "visible" }),
    document: {
      readyState: "complete",
      set title(value) { messages.push(value.split(':')[1]); },
      querySelector: (selector) => {
        if (selector.includes("sign_out")) return state.signedIn ? button : null;
        if (selector === "header, nav") return state.header ? button : null;
        return state.challenge ? button : null;
      },
      querySelectorAll: (selector) => selector === "button, a"
        ? [...(shadow ? [] : [button]), ...(state.loggedOut ? [{ ...button, textContent: "Log in" }] : [])]
        : selector === "*" ? (shadow ? [host] : [])
        : state.consent ? [{ ...button, tagName: "IFRAME" }] : [],
      // document-level hit testing only sees the shadow host, never the button inside it
      elementFromPoint: () => state.blocked ? {} : shadow ? host : button,
    },
    setInterval: (fn) => { tick = fn; }, setTimeout: (fn) => { timeout = fn; },
    clearInterval() {}, clearTimeout() {},
  });
  return { state, button, navigations, messages, advance: (ms) => { now += ms; tick(); }, tick: () => tick(), timeout: () => timeout(), clicks: () => clicks };
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
  assert.deepEqual(page.messages, ['show']);
  assert.deepEqual(page.navigations, []);
});
test('leaves authentication pages untouched, including the fallback timer', () => {
  const page = run({ login: true });
  page.tick(); page.timeout();
  assert.equal(page.clicks(), 0);
  assert.deepEqual(page.messages, []);
  assert.deepEqual(page.navigations, []);
});
test('does not infer login from a homepage redirect', () => {
  const page = run({ offFile: true });
  assert.equal(page.clicks(), 0);
  assert.deepEqual(page.navigations, []);
});

test('waits for cookie consent and resumes without consuming the download click', () => {
  const page = run({ consent: true, blocked: true });
  assert.equal(page.clicks(), 0);
  assert.deepEqual(page.messages, ['show']);
  assert.deepEqual(page.navigations, []);
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
  assert.deepEqual(page.messages, ['signed-out', 'show']);
  assert.deepEqual(page.navigations, []);
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
  assert.deepEqual(page.messages, ['signed-in']);
  assert.deepEqual(page.navigations, []);
});

test('recognizes a completed sign-in return without specific avatar markup', () => {
  const page = run({ offFile: true, loginReturn: true });
  assert.deepEqual(page.messages, []);
  page.advance(3000);
  assert.deepEqual(page.messages, ['signed-in']);
  assert.deepEqual(page.navigations, []);
});
test('does not accept a returned page still showing a login button', () => {
  const page = run({ offFile: true, loginReturn: true, loggedOut: true });
  page.advance(5000);
  assert.ok(!page.messages.includes('signed-in'));
  assert.ok(!page.messages.includes('signed-out'));
  assert.deepEqual(page.navigations, []);
  page.state.loggedOut = false;
  page.tick(); page.advance(3000);
  assert.ok(page.messages.includes('signed-in'));
});
test('does not accept a challenge or an unrendered navigation as a login', () => {
  for (const options of [{ challenge: true }, { header: false }, { consent: true }]) {
    const page = run({ offFile: true, loginReturn: true, ...options });
    page.advance(5000);
    assert.ok(!page.messages.includes('signed-in'));
    assert.deepEqual(page.navigations, []);
  }
});

test('finds and clicks Slow download inside a web component shadow root', () => {
  const page = run({ shadow: true });
  page.tick();
  assert.equal(page.clicks(), 1);
  assert.deepEqual(page.messages, []);
  assert.deepEqual(page.navigations, []);
});
test('does not click a shadow-root button covered by an overlay', () => {
  const page = run({ shadow: true, blocked: true });
  assert.equal(page.clicks(), 0);
  assert.deepEqual(page.messages, ['show']);
  page.state.blocked = false; page.tick();
  assert.equal(page.clicks(), 1);
});

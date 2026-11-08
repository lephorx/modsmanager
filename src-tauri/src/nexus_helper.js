// Injected only into Nexus pages. The persistent webview keeps the user's cookies.
(() => {
  const info = __MM_INFO__;
  if (window.__mmNexusCleanup) window.__mmNexusCleanup();
  let clicked = false;
  let prompted = false;
  let reportedLogin = null;
  const reportLogin = (value) => {
    if (reportedLogin === value) return;
    reportedLogin = value;
    signal(value ? "signed-in" : "signed-out");
  };
  const signal = (action) => { location.href = `mm-nexus://${action}`; };
  const show = () => {
    if (!prompted) { prompted = true; signal("show"); }
  };
  const visible = (el) => el.getClientRects().length && getComputedStyle(el).visibility !== "hidden";
  const accessible = (el) => {
    el.scrollIntoView({ block: "center" });
    const rect = el.getBoundingClientRect();
    const top = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
    return top === el || el.contains(top);
  };
  const onHome = () => ["/", "/games", "/games/"].includes(location.pathname);
  const controls = () => [...document.querySelectorAll("button, a")];
  const label = (el) => el.textContent.trim().replace(/\s+/g, " ").toLowerCase();
  const check = () => {
    if (location.hostname === "users.nexusmods.com" ||
        document.querySelector('input[type="password"], #challenge-running, #challenge-stage')) {
      show();
      if (location.hostname === "users.nexusmods.com") reportLogin(false);
      return;
    }
    // Consent can be in a cross-origin iframe. Leave the choice to the user and
    // resume when the overlay is gone; never consume the click behind a dialog.
    const consent = [...document.querySelectorAll('iframe[id^="sp_message_iframe"], [role="dialog"], [aria-modal="true"]')]
      .some((el) => visible(el) && (el.tagName === "IFRAME" || /consent|cookies/i.test(el.textContent)));
    if (consent) { show(); return; }
    const login = controls().find((el) => label(el) === "log in" && visible(el));
    if (login) { reportLogin(false); show(); return; }
    // Require positive account UI, never infer login merely from a missing button
    // while Nexus is still rendering. Header scope avoids mod-author profile links.
    const account = document.querySelector('a[href*="/sign_out"], a[href*="/logout"], header a[href*="/users/"], header [aria-label="User menu"], header [aria-label="Open user menu"], header [aria-label="Profile menu"]');
    if (account && visible(account)) reportLogin(true);
    if (onHome()) return; // Rust resumes the pending file after persisting login.
    if (clicked) return;
    const button = [...document.querySelectorAll("button, a")].find((el) =>
      el.textContent.trim().replace(/\s+/g, " ").toLowerCase() === "slow download" &&
      !el.disabled && el.getAttribute("aria-disabled") !== "true" && visible(el));
    if (!button) return;
    if (!accessible(button)) { show(); return; }
    clicked = true;
    // Do not navigate to a visibility signal before clicking: that can interrupt
    // the site's click handler. Rust hides the window when it receives the nxm link.
    button.click();
  };
  // Polling also notices disabled buttons becoming enabled, without depending on animation frames
  // (which are suspended by hidden webviews). Never repeat a click during the site's countdown.
  const timer = setInterval(check, 500);
  const fallback = setTimeout(show, 45000);
  window.__mmNexusCleanup = () => { clearInterval(timer); clearTimeout(fallback); };
  check();
})();

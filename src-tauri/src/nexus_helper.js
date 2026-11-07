// Injected only into Nexus pages. The persistent webview keeps the user's cookies.
(() => {
  const info = __MM_INFO__;
  if (window.__mmNexusCleanup) window.__mmNexusCleanup();
  let clicked = false;
  let prompted = false;
  const signal = (action) => { location.href = `mm-nexus://${action}`; };
  const show = () => {
    if (!prompted) { prompted = true; signal("show"); }
  };
  const target = new URL(info.url);
  const onFile = () => location.pathname === target.pathname &&
    new URLSearchParams(location.search).get("file_id") === target.searchParams.get("file_id");
  const check = () => {
    if (location.hostname === "users.nexusmods.com" ||
        document.querySelector('input[type="password"], #challenge-running, #challenge-stage')) {
      show();
      return;
    }
    // Nexus may return to the homepage after login.
    if (!onFile()) { location.replace(info.url); return; }
    if (clicked) return;
    const button = [...document.querySelectorAll("button, a")].find((el) =>
      el.textContent.trim().replace(/\s+/g, " ").toLowerCase() === "slow download" &&
      !el.disabled && el.getAttribute("aria-disabled") !== "true" && el.getClientRects().length);
    if (!button) return;
    clicked = true;
    signal("hide");
    button.click();
  };
  // Polling also notices disabled buttons becoming enabled, without depending on animation frames
  // (which are suspended by hidden webviews). Never repeat a click during the site's countdown.
  const timer = setInterval(check, 500);
  const fallback = setTimeout(show, 45000);
  window.__mmNexusCleanup = () => { clearInterval(timer); clearTimeout(fallback); };
  check();
})();

// UDM extension glue: shows the "Download with UDM" button over a playing
// <video>/<audio>. Whether a source qualifies is decided in Rust (the
// background asks udm.decide); this file only forwards and draws.
(() => {
  const api = globalThis.browser ?? globalThis.chrome;
  let host = null;
  let target = null;

  function place() {
    if (!host || !target) return;
    const r = target.getBoundingClientRect();
    host.style.top = `${Math.max(0, r.top) + 8}px`;
    host.style.left = `${Math.max(0, r.right - host.offsetWidth - 8)}px`;
  }

  function hide() {
    host?.remove();
    host = null;
    target = null;
  }

  function show(el, src, label) {
    hide();
    host = document.createElement("div");
    host.style.cssText = "position:fixed;z-index:2147483647;top:0;left:0";
    const button = document.createElement("button");
    button.textContent = label;
    button.style.cssText =
      "font:13px system-ui,sans-serif;padding:6px 10px;border:0;border-radius:6px;" +
      "background:#1C4FBF;color:#fff;cursor:pointer;box-shadow:0 2px 6px rgba(0,0,0,.35)";
    button.addEventListener("click", async (e) => {
      e.preventDefault();
      e.stopPropagation();
      button.disabled = true;
      const text = await api.runtime.sendMessage({ udm: "media-click", src });
      button.textContent = text ?? label;
      setTimeout(hide, 2500);
    });
    host.attachShadow({ mode: "closed" }).append(button);
    document.documentElement.append(host);
    target = el;
    place();
  }

  document.addEventListener(
    "play",
    async (e) => {
      const el = e.target;
      if (!(el instanceof HTMLMediaElement) || el === target) return;
      const src = el.currentSrc || el.src;
      const label = await api.runtime.sendMessage({ udm: "media-probe", src });
      if (label) show(el, src, label);
    },
    true,
  );
  document.addEventListener("ended", (e) => e.target === target && hide(), true);
  addEventListener("scroll", place, true);
  addEventListener("resize", place);
})();

// Expects PROMPT (string) and TIMEOUT_MS (number) to be defined before this script.
new Promise((resolve) => {
  const box = document.createElement("div");
  box.style.cssText = "position:fixed;pointer-events:none;z-index:2147483647;border:2px solid #f0883e;" +
    "background:rgba(240,136,62,.15);border-radius:3px;display:none";
  const banner = document.createElement("div");
  banner.style.cssText = "position:fixed;top:8px;left:50%;transform:translateX(-50%);z-index:2147483647;" +
    "background:#24292f;color:#fff;font:14px system-ui;padding:8px 14px;border-radius:6px;pointer-events:none";
  banner.textContent = `${PROMPT || "Click an element for the agent"}  (Esc to cancel)`;
  document.documentElement.append(box, banner);
  let current = null;
  const move = (e) => {
    current = e.target;
    const r = current.getBoundingClientRect();
    Object.assign(box.style, { display: "block", left: r.left + "px", top: r.top + "px", width: r.width + "px", height: r.height + "px" });
  };
  const stop = (e) => { e.preventDefault(); e.stopPropagation(); e.stopImmediatePropagation(); };
  const click = (e) => {
    stop(e);
    const el = e.target;
    if (!el.dataset.herdrRef) el.setAttribute("data-herdr-ref", String(__hb.nextRef()));
    done({ ref: +el.dataset.herdrRef, role: __hb.role(el), name: __hb.name(el), selector: __hb.selector(el),
      text: (el.innerText || "").trim().slice(0, 500) });
  };
  const key = (e) => { if (e.key === "Escape") { stop(e); done({ cancelled: true }); } };
  const timer = setTimeout(() => done({ timeout: true }), TIMEOUT_MS);
  const opts = { capture: true };
  function done(v) {
    clearTimeout(timer);
    removeEventListener("mousemove", move, opts);
    removeEventListener("click", click, opts);
    removeEventListener("mousedown", stop, opts);
    removeEventListener("keydown", key, opts);
    box.remove();
    banner.remove();
    resolve(v);
  }
  addEventListener("mousemove", move, opts);
  addEventListener("mousedown", stop, opts);
  addEventListener("click", click, opts);
  addEventListener("keydown", key, opts);
})

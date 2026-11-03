// Runs in every page (main world, before the page's own scripts) and keeps the console
// output, uncaught errors and fetch/XHR results that `herdr-web-browser console` and
// `network` read back. Page code can tamper with these: they're untrusted, like the page.
(() => {
  if (window.__herdrLog) return;
  const MAX = 500;
  const log = (window.__herdrLog = []);
  const net = (window.__herdrNet = []);
  const push = (list, entry) => {
    list.push({ t: Date.now(), ...entry });
    if (list.length > MAX) list.shift();
  };
  const show = (v) => {
    if (v instanceof Error) return `${v.name}: ${v.message}`;
    if (typeof v === "string") return v;
    try {
      return JSON.stringify(v) ?? String(v);
    } catch {
      return String(v);
    }
  };

  for (const level of ["log", "info", "warn", "error", "debug"]) {
    const original = console[level];
    console[level] = function (...args) {
      const err = args.find((a) => a instanceof Error);
      push(log, { level, text: args.map(show).join(" "), stack: err?.stack });
      return original.apply(this, args);
    };
  }
  addEventListener("error", (e) => {
    if (e.target === window) {
      push(log, { level: "exception", text: e.message, stack: e.error?.stack || `${e.filename}:${e.lineno}:${e.colno}` });
    } else if (e.target?.src || e.target?.href) {
      // A <script>, <img> or <link> that failed to load (these don't bubble, hence capture).
      push(net, { method: "GET", url: e.target.src || e.target.href, status: 0, error: `<${e.target.tagName.toLowerCase()}> failed to load` });
    }
  }, true);
  addEventListener("unhandledrejection", (e) => {
    push(log, { level: "exception", text: `Unhandled promise rejection: ${show(e.reason)}`, stack: e.reason?.stack });
  });

  const fetch = window.fetch;
  window.fetch = async function (input, init) {
    const url = String(input?.url ?? input);
    const method = String(init?.method || input?.method || "GET").toUpperCase();
    try {
      const res = await fetch.apply(this, arguments);
      push(net, { method, url: res.url || url, status: res.status, type: "fetch" });
      return res;
    } catch (err) {
      push(net, { method, url, status: 0, type: "fetch", error: show(err) });
      throw err;
    }
  };
  const { open, send } = XMLHttpRequest.prototype;
  XMLHttpRequest.prototype.open = function (method, url) {
    this.__herdr = { method: String(method).toUpperCase(), url: String(url) };
    return open.apply(this, arguments);
  };
  XMLHttpRequest.prototype.send = function () {
    this.addEventListener("loadend", () => {
      if (this.__herdr) push(net, { ...this.__herdr, status: this.status, type: "xhr", error: this.status ? undefined : "request failed" });
    });
    return send.apply(this, arguments);
  };
  performance.setResourceTimingBufferSize(2000);
})();

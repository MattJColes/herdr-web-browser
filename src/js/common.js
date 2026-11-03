// Shared helpers, prepended to every injected script.
const __hb = {
  visible(el) {
    if (!el.isConnected) return false;
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) return false;
    const cs = getComputedStyle(el);
    return cs.visibility !== "hidden" && cs.display !== "none";
  },
  name(el) {
    const t = (s) => (s || "").replace(/\s+/g, " ").trim();
    const labels = el.labels ? [...el.labels].map((l) => l.innerText).join(" ") : "";
    return t(
      el.getAttribute("aria-label") || labels || el.getAttribute("alt") || el.getAttribute("placeholder") ||
      el.innerText || el.getAttribute("title") || el.value || el.getAttribute("name") || ""
    ).slice(0, 80);
  },
  role(el) {
    const r = el.getAttribute("role");
    if (r) return r;
    const tag = el.tagName.toLowerCase();
    if (tag === "a") return "link";
    if (tag === "input") return (el.type || "text") === "text" ? "textbox" : el.type;
    if (tag === "textarea") return "textbox";
    if (tag === "select") return "combobox";
    if (el.isContentEditable) return "textbox";
    return tag;
  },
  nextRef() {
    let max = 0;
    for (const e of document.querySelectorAll("[data-herdr-ref]")) max = Math.max(max, +e.dataset.herdrRef || 0);
    return max + 1;
  },
  selector(el) {
    const parts = [];
    for (let e = el; e && e.nodeType === 1 && parts.length < 6; e = e.parentElement) {
      if (e.id && document.querySelectorAll("#" + CSS.escape(e.id)).length === 1) {
        parts.unshift("#" + CSS.escape(e.id));
        break;
      }
      const tag = e.tagName.toLowerCase();
      const sibs = e.parentElement ? [...e.parentElement.children].filter((c) => c.tagName === e.tagName) : [];
      parts.unshift(sibs.length > 1 ? `${tag}:nth-of-type(${sibs.indexOf(e) + 1})` : tag);
    }
    return parts.join(" > ");
  },
  resolve(target) {
    const sel = /^\d+$/.test(target) ? `[data-herdr-ref="${target}"]` : target;
    const el = document.querySelector(sel);
    if (!el) throw new Error(/^\d+$/.test(target) ? `no element [${target}]; run snapshot again` : `no element matches ${target}`);
    return el;
  },
};

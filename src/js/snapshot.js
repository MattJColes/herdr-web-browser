(() => {
  const SEL = 'a[href],button,input:not([type=hidden]),select,textarea,summary,[contenteditable=""],[contenteditable=true],' +
    '[onclick],[role=button],[role=link],[role=checkbox],[role=radio],[role=switch],[role=tab],[role=menuitem],' +
    '[role=option],[role=textbox],[role=combobox],[role=searchbox]';
  for (const e of document.querySelectorAll("[data-herdr-ref]")) e.removeAttribute("data-herdr-ref");
  const items = [];
  for (const el of document.querySelectorAll(SEL)) {
    if (items.length >= 300 || !__hb.visible(el)) continue;
    const ref = items.length + 1;
    el.setAttribute("data-herdr-ref", String(ref));
    const item = { ref, role: __hb.role(el), name: __hb.name(el) };
    if (el.tagName === "A") item.href = el.href;
    if ("value" in el && el.value && el.tagName !== "BUTTON") item.value = String(el.value).slice(0, 80);
    if (el.disabled) item.disabled = true;
    items.push(item);
  }
  return { url: location.href, title: document.title, items };
})()

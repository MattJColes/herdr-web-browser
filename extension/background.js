// Talks only to the native host registered for this extension's pinned ID;
// web pages cannot reach it, so a page cannot push prompts into an agent.
const HOST = "dev.herdr.browser";
const MENUS = [
  ["selection", "Send selection to herdr agent", ["selection"]],
  ["page", "Send page to herdr agent", ["page"]],
  ["link", "Send link to herdr agent", ["link"]],
  ["screenshot", "Send screenshot to herdr agent", ["page", "selection", "link", "image"]],
];

chrome.contextMenus.removeAll(() => {
  for (const [id, title, contexts] of MENUS) chrome.contextMenus.create({ id, title, contexts });
});

async function grab(kind, tab, info) {
  if (kind === "screenshot") {
    return { data_url: await chrome.tabs.captureVisibleTab(tab.windowId, { format: "png" }) };
  }
  if (kind === "link") return { text: info.linkUrl };
  const [{ result }] = await chrome.scripting.executeScript({
    target: { tabId: tab.id },
    func: (k) => (k === "selection" ? String(getSelection()) : document.body.innerText),
    args: [kind],
  });
  return { text: result };
}

function badge(tabId, ok, error) {
  chrome.action.setBadgeBackgroundColor({ tabId, color: ok ? "#1a7f37" : "#cf222e" });
  chrome.action.setBadgeText({ tabId, text: ok ? "✓" : "✗" });
  chrome.action.setTitle({ tabId, title: ok ? "Sent to herdr agent" : `herdr-web-browser: ${error}` });
  setTimeout(() => chrome.action.setBadgeText({ tabId, text: "" }), 4000);
}

async function send(kind, tab, info = {}) {
  try {
    const msg = { kind, url: tab.url, title: tab.title, ...(await grab(kind, tab, info)) };
    const res = await chrome.runtime.sendNativeMessage(HOST, msg);
    badge(tab.id, res?.ok, res?.error);
  } catch (e) {
    badge(tab.id, false, e?.message ?? String(e));
  }
}

chrome.contextMenus.onClicked.addListener((info, tab) => send(info.menuItemId, tab, info));
chrome.action.onClicked.addListener((tab) => send("page", tab));

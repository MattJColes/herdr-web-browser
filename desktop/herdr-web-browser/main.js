// @ts-check
// herdr-web-browser menu (mod.open:herdr-web-browser): acts for the agent that had focus.
import { View, div } from "gpui-kit";
import { v_flex } from "gpui-base";
import { exec, agents, focused, props, closeOverlay, notify, theme, run } from "herdr";

const BIN = "__HERDR_BROWSER_BIN__";

export default class Browser extends View {
  init(_props, _cx) {
    const f = focused();
    const list = agents();
    const agent = list.find((a) => a.address === f?.terminal) ?? list.find((a) => a.address === f?.agent);
    this.agent = agent ? agent.address : null;
    this.busy = null;
  }

  act(label, args, cx) {
    this.busy = label;
    cx.notify();
    cx.spawn(async (_cx) => {
      const r = await exec(BIN, args).catch((e) => ({ code: 1, stdout: "", stderr: String(e) }));
      const out = (r.code === 0 ? r.stdout : r.stderr).trim().slice(0, 240);
      notify({ title: "herdr-web-browser", body: out || label, level: r.code === 0 ? "success" : "error" });
      closeOverlay(props().overlay);
    });
  }

  row(t, id, label, args) {
    return div().id(id).px(12).py(7).rounded(6).cursor_pointer()
      .hover((s) => s.bg(t.hover))
      .on_click((_e, cx) => this.act(label, args, cx))
      .child(label);
  }

  render(_cx) {
    const t = theme();
    const a = this.agent;
    const owner = a ? ["--owner", a] : [];
    const to = a ? ["--agent", a] : [];
    const rows = [
      a && this.row(t, "hb-show", `Open live view beside ${a}`, [...owner, "show"]),
      this.row(t, "hb-page", "Send page to agent", [...owner, "send", "page", ...to]),
      this.row(t, "hb-sel", "Send selection to agent", [...owner, "send", "selection", ...to]),
      this.row(t, "hb-shot", "Send screenshot to agent", [...owner, "send", "screenshot", ...to]),
      this.row(t, "hb-mode", "Toggle headed / headless", ["mode"]),
      this.row(t, "hb-stop", "Stop the browser", ["stop"]),
      div().id("hb-settings").px(12).py(7).rounded(6).cursor_pointer().hover((s) => s.bg(t.hover))
        .on_click(() => { closeOverlay(props().overlay); run("mod.open", { name: "herdr-web-browser-settings" }); })
        .child("Settings…"),
    ].filter(Boolean);
    return v_flex().id("root").size_full().bg(t.surface).p(6).gap(1).text_size(13).text_color(t.foreground)
      .child(div().px(12).py(6).text_color(t.muted).child(this.busy ? `${this.busy}…` : a ? `Browser · ${a}` : "Browser · no agent focused"))
      .children(rows);
  }
}

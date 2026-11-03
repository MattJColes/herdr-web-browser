// @ts-check
// Opened by herdr-web-browser when a send has no obvious target; the choice becomes sticky.
import { View, div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { exec, agents, props, closeOverlay, theme } from "herdr";

const BIN = "__HERDR_BROWSER_BIN__";

export default class Picker extends View {
  init(_props, _cx) {
    this.chosen = false;
  }

  choose(address, cx) {
    this.chosen = true;
    cx.spawn(async (_cx) => {
      await exec(BIN, ["picked", address]).catch(() => null);
      closeOverlay(props().overlay);
    });
  }

  async close(_cx) {
    if (!this.chosen) await exec(BIN, ["picked"]).catch(() => null);
  }

  render(_cx) {
    const t = theme();
    const list = agents();
    return v_flex().id("root").size_full().bg(t.surface).p(6).gap(1).text_size(13).text_color(t.foreground)
      .child(div().px(12).py(6).text_color(t.muted).child("Send to which agent? (Esc to cancel)"))
      .children(list.map((a) =>
        h_flex().id(`hb-pick-${a.address}`).px(12).py(7).gap(10).rounded(6).cursor_pointer()
          .hover((s) => s.bg(t.hover))
          .on_click((_e, cx) => this.choose(a.address, cx))
          .child(div().w(90).child(a.address))
          .child(div().w(110).text_color(t.muted).child(a.app ?? ""))
          .child(div().w(70).text_color(t.muted).child(a.status ?? ""))
          .child(div().flex_1().truncate().child(`${a.space_name} · ${a.tab_name}`))))
      .when(list.length === 0, (el) => el.child(div().px(12).text_color(t.dim).child("No agents running")));
  }
}

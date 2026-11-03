// @ts-check
// The CLI can't post desktop notifications; it stashes one and opens this, which posts it and closes.
import { View, div } from "gpui-kit";
import { exec, props, closeOverlay, notify } from "herdr";

const BIN = "__HERDR_BROWSER_BIN__";

export default class Toast extends View {
  init(_props, cx) {
    cx.spawn(async (_cx) => {
      const r = await exec(BIN, ["--json", "take-toast"]).catch(() => null);
      try {
        const toast = r && r.code === 0 ? JSON.parse(r.stdout).toast : null;
        if (toast) notify({ title: toast[0], body: toast[1], level: toast[0].includes("failed") ? "error" : "success" });
      } finally {
        closeOverlay(props().overlay);
      }
    });
  }

  render(_cx) {
    return div();
  }
}

// @ts-check
// Browser settings (the live view's cog): set up agents, and copy chosen Google Chrome
// sign-ins into the browser, picked from a searchable, tickable list of sites.
import { View, div } from "gpui-kit";
import { v_flex, h_flex, InputState, Input, Checkbox } from "gpui-base";
import { exec, notify, theme } from "herdr";

const BIN = "__HERDR_BROWSER_BIN__";

export default class Settings extends View {
  init(_props, cx) {
    this.sites = undefined; // undefined: loading; [{ site, cookies }] once read
    this.error = "";
    this.picked = new Set();
    this.query = "";
    this.confirming = false;
    this.busy = "";
    this.search = InputState.new({ placeholder: "Search sites" });
    this.search.on("change", (_e, cx) => {
      this.query = this.search.value().trim().toLowerCase();
      this.confirming = false;
      cx.notify();
    });
    this.searchFocused = true;
    this.search.on("focus", (_e, cx) => { this.searchFocused = true; cx.notify(); });
    this.search.on("blur", (_e, cx) => { this.searchFocused = false; cx.notify(); });
    // Like herdr's own dialogs: focus now, and again once the overlay has settled.
    this.search.focus();
    cx.timer.after(600, () => this.search.focus());
    cx.spawn(async (cx) => {
      const r = await exec(BIN, ["--json", "import-logins", "--list"]).catch((e) => ({ code: 1, stdout: "", stderr: String(e) }));
      if (r.code === 0) this.sites = JSON.parse(r.stdout);
      else this.error = r.stderr.trim() || "Couldn't read Google Chrome's sign-ins";
      cx.notify();
    });
  }

  shown() {
    return (this.sites ?? []).filter((s) => s.site.includes(this.query));
  }

  toggle(site, on, cx) {
    if (on) this.picked.add(site);
    else this.picked.delete(site);
    this.confirming = false;
    cx.notify();
  }

  toggleShown(cx) {
    const shown = this.shown();
    const all = shown.length > 0 && shown.every((s) => this.picked.has(s.site));
    for (const s of shown) all ? this.picked.delete(s.site) : this.picked.add(s.site);
    this.confirming = false;
    cx.notify();
  }

  act(label, args, cx) {
    this.busy = label;
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await exec(BIN, args).catch((e) => ({ code: 1, stdout: "", stderr: String(e) }));
      const out = (r.code === 0 ? r.stdout : r.stderr).trim().slice(0, 300);
      notify({ title: "Browser settings", body: out || label, level: r.code === 0 ? "success" : "error" });
      this.busy = "";
      this.confirming = false;
      cx.notify();
    });
  }

  importPicked(cx) {
    // Two clicks: the first shows what importing means.
    if (!this.confirming) {
      this.confirming = true;
      cx.notify();
      return;
    }
    const all = this.sites && this.picked.size === this.sites.length;
    this.act("Importing sign-ins", ["import-logins", ...(all ? ["--all"] : [...this.picked])], cx);
  }

  button(t, id, label, onClick, enabled = true, strong = false) {
    return div().id(id).px(12).py(6).rounded(6).flex_none()
      .bg(strong && enabled ? t.accent : t.hover).text_color(strong && enabled ? t.background : enabled ? t.foreground : t.dim)
      .when(enabled, (el) => el.cursor_pointer().on_click((_e, cx) => onClick(cx)))
      .child(label);
  }

  siteRow(t, s) {
    const on = this.picked.has(s.site);
    return Checkbox.new(`site-${s.site}`).checked(on).on_change((v, cx) => this.toggle(s.site, v, cx))
      .child(h_flex().px(10).py(5).gap(10).rounded(5).items_center().cursor_pointer().hover((st) => st.bg(t.hover))
        .child(div().w(15).h(15).rounded(3).border(1).border_color(on ? t.accent : t.border).bg(on ? t.accent : t.background)
          .text_size(11).text_color(t.background).flex().items_center().justify_center().child(on ? "✓" : ""))
        .child(div().flex_1().truncate().child(s.site))
        .child(div().text_color(t.dim).child(`${s.cookies} cookie${s.cookies === 1 ? "" : "s"}`)));
  }

  render(_cx) {
    const t = theme();
    const shown = this.shown();
    const n = this.picked.size;
    const allShown = shown.length > 0 && shown.every((s) => this.picked.has(s.site));
    const list = this.sites === undefined
      ? div().px(10).text_color(t.dim).child(this.error || "Reading Google Chrome's sign-ins…")
      : v_flex().flex_1().min_h(0).overflow_y_scrollbar().children(shown.map((s) => this.siteRow(t, s)))
        .when(shown.length === 0, (el) => el.child(div().px(10).py(6).text_color(t.dim).child("No matching sites")));
    return v_flex().id("root").size_full().bg(t.popover).p(14).gap(10).text_size(13).text_color(t.foreground)
      .child(div().text_size(15).child("Browser settings"))
      .child(h_flex().gap(10).items_center()
        .child(div().flex_1().text_color(t.muted).child("Teach Claude Code, Codex, OpenCode and Pi to open web pages here."))
        .child(this.button(t, "agents", "Set up agents", (cx) => this.act("Setting up agents", ["install-agents"], cx), !this.busy)))
      .child(div().h(1).bg(t.border))
      .child(div().child("Sign in with your Google Chrome logins"))
      .child(div().text_color(t.muted).child("Tick the sites to copy from Chrome. Every agent using this browser can then use them as you, so pick only what you're happy to share."))
      .child(Input.new(this.search).h(30).px(10).items_center().rounded(6).border(1).border_color(this.searchFocused ? t.accent : t.input).bg(t.background).text_color(t.foreground))
      .child(h_flex().gap(10).items_center().text_color(t.muted)
        .child(this.button(t, "all", allShown ? "Untick shown" : "Tick shown", (cx) => this.toggleShown(cx), shown.length > 0))
        .child(div().flex_1().child(this.sites ? `${n} of ${this.sites.length} sites ticked` : "")))
      .child(list)
      .child(h_flex().gap(10).items_center()
        .child(div().flex_1().text_color(this.confirming ? t.foreground : t.dim).child(
          this.busy ? `${this.busy}…` : this.confirming ? `Agents will be able to act as you on ${n} site${n === 1 ? "" : "s"}. Click again to import.` : "macOS asks once to let it read Chrome's cookie key."))
        .child(this.button(t, "import", this.confirming ? "Import" : `Import ${n} site${n === 1 ? "" : "s"}`, (cx) => this.importPicked(cx), n > 0 && !this.busy, true)));
  }
}

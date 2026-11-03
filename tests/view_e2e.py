#!/usr/bin/env python3
"""Click every live-view control through a real pty and check its effect.

Runs `herdr-web-browser view` in a pseudo-terminal, sends SGR mouse clicks at each
button's cell, and verifies the result with the CLI. Fake `hc` / `herdr-app`
on PATH stand in for herdr desktop, so sends are logged instead of typed into
a real agent. The browser it drives is the real one (headless, own state dir).

    python3 -I tests/view_e2e.py [path/to/herdr-web-browser]
    E2E_WINDOW=1 python3 -I tests/view_e2e.py   # also the window button (opens Chrome)
"""
import fcntl
import json
import os
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "target/release/herdr-web-browser")
COLS, ROWS = 100, 40
OWNER = "t99"

tmp = tempfile.mkdtemp(prefix="hb-e2e-")
fake = os.path.join(tmp, "bin")
os.makedirs(fake)
log = os.path.join(tmp, "calls.log")
agents = {"spaces": [{"id": "sp1", "name": "e2e", "active": True, "tabs": [{"terminals": [
    {"terminal": OWNER, "pane": "pe2e", "status": "idle", "app": "claude-code"}]}]}]}
for name, body in {
    "hc": f'echo "hc $*" >> {log}\ncase "$1" in ls) echo \'{{"terminals":[{{"address":"{OWNER}","state":"live","status":"idle"}}]}}\';; *) echo \'{{}}\';; esac\n',
    "herdr-app": f"echo \"herdr-app $*\" >> {log}\ncase \"$1\" in ls) echo '{json.dumps(agents)}';; *) echo '{{}}';; esac\n",
}.items():
    path = os.path.join(fake, name)
    with open(path, "w") as f:
        f.write("#!/bin/sh\n" + body)
    os.chmod(path, 0o755)

env = {k: v for k, v in os.environ.items() if not k.startswith(("HC_", "HERDR_"))}
env.update(PATH=f"{fake}:{env['PATH']}", HERDR_PLUGIN_STATE_DIR=os.path.join(tmp, "state"),
           HERDR_PLUGIN_ROOT=ROOT, HERDR_PLUGIN_CONFIG_DIR=os.path.join(tmp, "cfg"))


def cli(*args):
    r = subprocess.run([BIN, "--owner", OWNER, "--json", *args], env=env, capture_output=True, text=True, timeout=60, check=False)
    if r.returncode:
        raise SystemExit(f"cli {args}: {r.stderr}")
    return json.loads(r.stdout) if r.stdout.strip() else None


def state():
    with open(os.path.join(tmp, "state", "state.json")) as f:
        return json.load(f)


def url():
    return cli("eval", "location.href")


def calls():
    return open(log).read() if os.path.exists(log) else ""


failures = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + name + (f"  ({detail})" if detail and not ok else ""))
    if not ok:
        failures.append(name)


cli("start", "--headless")
cli("open", "data:text/html,<title>A</title><p id=p>page A text</p>")
cli("open", "data:text/html,<title>B</title><p>page B text</p>")

pid, fd = pty.fork()
if pid == 0:
    os.execve(BIN, [BIN, "--owner", OWNER, "view"], env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))


screen = bytearray()


def pump(seconds):
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                screen.extend(os.read(fd, 1 << 16))
            except OSError:
                return False
    return True


def last_status():
    """Text of the most recent toolbar status (messages contain these words)."""
    import re
    text = re.sub(rb"\x1b(\[[0-9;?<]*[a-zA-Z]|_G[^\x1b]*\x1b\\|\][^\x07]*\x07)", b"", bytes(screen)).decode("utf8", "replace")
    hits = re.findall(r"(error[^\u2715]{0,160}|\u2713 [^\u2715]{0,80}|view lost[^\u2715]{0,120})", text)
    return hits[-3:]


def click(col, row):
    os.write(fd, f"\x1b[<0;{col + 1};{row + 1}M".encode())
    os.write(fd, f"\x1b[<0;{col + 1};{row + 1}m".encode())
    pump(2.5)


pump(4)  # detection timeout, attach, first frames

click(2, 0)
check("back", url().startswith("data:text/html,<title>A"), url())
click(6, 0)
check("forward", url().startswith("data:text/html,<title>B"), url())
cli("eval", "window.__mark = 1")
click(10, 0)
check("reload", cli("eval", "window.__mark === undefined") is True)

click(30, 0)
os.write(fd, b"data:text/html,<title>C</title>typed\r")
pump(3)
check("address bar", url().startswith("data:text/html,<title>C"), url())

def key(b):
    os.write(fd, b)
    pump(2.5)


# Header, right of the address bar: screenshot then window, 4 columns each, then a gap.
SHOT_COL, MODE_COL = COLS - 8, COLS - 4

cli("eval", "getSelection().selectAllChildren(document.body)")
before = calls()
key(b"\x07")  # ctrl+g
check("send page", "hc send t99 [herdr-web-browser] page" in calls()[len(before):], calls()[len(before):][-200:])
before = calls()
key(b"\x13")  # ctrl+s
check("send selection", "hc send t99 [herdr-web-browser] selection" in calls()[len(before):])
before = calls()
click(SHOT_COL, 0)
check("screenshot", "hc send t99 [herdr-web-browser] screenshot" in calls()[len(before):])

mine = state()["tabs"][OWNER]
key(b"\x0e")  # ctrl+n
check("next tab", state()["tabs"][OWNER] != mine)
key(b"\x10")  # ctrl+p
check("previous tab", state()["tabs"][OWNER] == mine)

# Pops a real Chrome window up over whatever you're doing, so only on request.
if os.environ.get("E2E_WINDOW"):
    click(MODE_COL, 0)
    pump(6)
    check("mode switch", state()["mode"] == "headed", state().get("mode"))
    click(MODE_COL, 0)
    pump(6)
    check("mode switch back", state()["mode"] == "headless", state().get("mode"))

os.write(fd, b"\x11")  # ctrl+q
pump(1)
done, status = os.waitpid(pid, os.WNOHANG)
check("close", done == pid)
if done != pid:
    os.kill(pid, 9)

cli("stop")
if failures:
    print("view said:", last_status())
print(f"\n{len(failures)} failed" if failures else "\nall passed")
sys.exit(1 if failures else 0)

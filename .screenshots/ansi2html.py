#!/usr/bin/env python3
"""Convert a tmux ANSI capture into a styled terminal-window HTML page.
Usage: ansi2html.py in.ansi out.html [title]
"""
import html
import re
import sys

# Tokyo-Night-ish palette for the 16 ANSI colours.
PALETTE = [
    "#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6",
    "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5",
]
DEFAULT_FG = "#c0caf5"
DEFAULT_BG = "#1a1b26"


def xterm256(n):
    if n < 16:
        return PALETTE[n]
    if n < 232:
        n -= 16
        r, g, b = n // 36, (n // 6) % 6, n % 6
        f = lambda v: 0 if v == 0 else 55 + 40 * v
        return "#%02x%02x%02x" % (f(r), f(g), f(b))
    v = 8 + (n - 232) * 10
    return "#%02x%02x%02x" % (v, v, v)


class State:
    def __init__(self):
        self.reset()

    def reset(self):
        self.fg = None
        self.bg = None
        self.bold = self.dim = self.italic = self.underline = self.reverse = False

    def key(self):
        return (self.fg, self.bg, self.bold, self.dim, self.italic, self.underline, self.reverse)

    def css(self):
        fg, bg = self.fg, self.bg
        if self.reverse:
            fg, bg = (bg or DEFAULT_BG), (fg or DEFAULT_FG)
        s = []
        if fg:
            s.append("color:" + fg)
        if bg:
            s.append("background:" + bg)
        if self.bold:
            s.append("font-weight:700")
        if self.dim:
            s.append("opacity:.6")
        if self.italic:
            s.append("font-style:italic")
        if self.underline:
            s.append("text-decoration:underline")
        return ";".join(s)

    def apply(self, params):
        i = 0
        while i < len(params):
            p = params[i]
            if p == 0:
                self.reset()
            elif p == 1:
                self.bold = True
            elif p == 2:
                self.dim = True
            elif p == 3:
                self.italic = True
            elif p == 4:
                self.underline = True
            elif p == 7:
                self.reverse = True
            elif p == 22:
                self.bold = self.dim = False
            elif p == 23:
                self.italic = False
            elif p == 24:
                self.underline = False
            elif p == 27:
                self.reverse = False
            elif 30 <= p <= 37:
                self.fg = PALETTE[p - 30]
            elif 90 <= p <= 97:
                self.fg = PALETTE[p - 90 + 8]
            elif 40 <= p <= 47:
                self.bg = PALETTE[p - 40]
            elif 100 <= p <= 107:
                self.bg = PALETTE[p - 100 + 8]
            elif p == 39:
                self.fg = None
            elif p == 49:
                self.bg = None
            elif p in (38, 48):
                target = "fg" if p == 38 else "bg"
                if i + 1 < len(params) and params[i + 1] == 5 and i + 2 < len(params):
                    setattr(self, target, xterm256(params[i + 2]))
                    i += 2
                elif i + 1 < len(params) and params[i + 1] == 2 and i + 4 < len(params):
                    r, g, b = params[i + 2 : i + 5]
                    setattr(self, target, "#%02x%02x%02x" % (r, g, b))
                    i += 4
            i += 1


SGR = re.compile(r"\x1b\[([0-9;:]*)m")


def render_line(line, st):
    out, pos, buf = [], 0, []

    def flush():
        if buf:
            text = html.escape("".join(buf))
            css = st.css()
            out.append(f'<span style="{css}">{text}</span>' if css else text)
            buf.clear()

    for m in SGR.finditer(line):
        buf.append(line[pos : m.start()])
        flush()
        params = [int(x) for x in re.split(r"[;:]", m.group(1)) if x != ""] or [0]
        st.apply(params)
        pos = m.end()
    buf.append(line[pos:])
    flush()
    return "".join(out)


def main():
    src, dst = sys.argv[1], sys.argv[2]
    title = sys.argv[3] if len(sys.argv) > 3 else "zdb"
    raw = open(src, encoding="utf-8").read().rstrip("\n")
    # strip any other escape sequences we do not understand
    raw = re.sub(r"\x1b\[[0-9;:?]*[A-Za-ln-z]", "", raw)
    st = State()
    lines = [render_line(l, st) for l in raw.split("\n")]
    body = "\n".join(lines)
    page = f"""<!doctype html><html><head><meta charset="utf-8"><style>
html,body{{margin:0;background:transparent}}
body{{padding:40px;display:inline-block}}
.win{{border-radius:12px;overflow:hidden;background:{DEFAULT_BG};
 box-shadow:0 20px 60px rgba(0,0,0,.55),0 0 0 1px rgba(255,255,255,.08)}}
.bar{{height:38px;background:#16161e;display:flex;align-items:center;padding:0 14px;position:relative}}
.dot{{width:12px;height:12px;border-radius:50%;margin-right:8px}}
.t{{position:absolute;left:0;right:0;text-align:center;color:#787c99;
 font:500 13px -apple-system,BlinkMacSystemFont,sans-serif}}
pre{{margin:0;padding:10px 14px 14px;color:{DEFAULT_FG};
 font:14px/1.3 Menlo,'SF Mono',monospace;white-space:pre}}
</style></head><body><div class="win">
<div class="bar"><span class="dot" style="background:#ff5f57"></span><span class="dot" style="background:#febc2e"></span><span class="dot" style="background:#28c840"></span><span class="t">{html.escape(title)}</span></div>
<pre>{body}</pre></div></body></html>"""
    open(dst, "w", encoding="utf-8").write(page)


main()

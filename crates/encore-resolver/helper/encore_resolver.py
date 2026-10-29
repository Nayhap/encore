# Encore stream resolver helper (D3).
#
# A resident process that speaks JSON lines over stdin/stdout. It loads
# yt-dlp as a library, takes cookies per request *in memory* (never a cookie
# file) and returns the chosen audio format. stdout carries protocol messages
# only; nothing secret is ever written to stderr.
#
# Request:  {"id": 1, "op": "resolve", "video_id": "...", "cookies": [...],
#            "js_runtime": {"name": "node", "path": "/path/to/node"}}
# Response: {"id": 1, "ok": true, "format": {...}} or
#           {"id": 1, "ok": false, "error": "...", "kind": "..."}

import http.cookiejar
import json
import re
import sys
import time
import traceback

PROTOCOL = 1

# The host speaks UTF-8 JSON. Python's pipes default to the system code page
# on Windows (cp1252, cp932...), which garbles or rejects non-ASCII paths such
# as a cache folder under C:\Users\José. -I ignores PYTHONIOENCODING, so set
# it here.
for _stream in (sys.stdin, sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding="utf-8", errors="replace")
    except Exception:
        pass

try:
    import yt_dlp
    from yt_dlp.cookies import YoutubeDLCookieJar
except Exception as e:  # pragma: no cover - reported to the host
    sys.stdout.write(json.dumps({"id": 0, "ok": False, "error": "yt-dlp is not installed: %s" % type(e).__name__, "kind": "unknown"}) + "\n")
    sys.stdout.flush()
    sys.exit(2)

# Signed URLs and cookie values must never leave this process except as the
# result value itself; scrub anything that looks like one from error text.
_SCRUB = [
    (re.compile(r"https?://\S+"), "<url>"),
    (re.compile(r"(?i)(SAPISID\w*|__Secure-\w+|SID|HSID|SSID|APISID|LOGIN_INFO)=\S+"), r"\1=<redacted>"),
]


def scrub(msg):
    msg = str(msg)
    for rx, rep in _SCRUB:
        msg = rx.sub(rep, msg)
    return msg[:400]


class QuietLogger:
    def __init__(self):
        self.errors = []

    def debug(self, msg):
        pass

    def info(self, msg):
        pass

    def warning(self, msg):
        pass

    def error(self, msg):
        self.errors.append(scrub(msg))


def classify(msg):
    m = msg.lower()
    if "not a bot" in m or "confirm you" in m or "captcha" in m or "429" in m:
        return "challenged"
    if "sign in" in m and ("age" in m or "inappropriate" in m):
        return "unavailable"
    if "sign in" in m or "login required" in m or "cookies are no longer valid" in m:
        return "sessionExpired"
    if any(k in m for k in ("unavailable", "private video", "not available", "removed", "blocked", "copyright", "premium", "country", "drm")):
        return "unavailable"
    if any(k in m for k in ("timed out", "timeout", "connection", "network", "resolve", "temporary failure", "ssl")):
        return "network"
    return "unknown"


def make_jar(cookies):
    jar = YoutubeDLCookieJar()
    for c in cookies or []:
        domain = c.get("domain") or ".youtube.com"
        jar.set_cookie(http.cookiejar.Cookie(
            version=0, name=c["name"], value=c["value"], port=None, port_specified=False,
            domain=domain, domain_specified=True, domain_initial_dot=domain.startswith("."),
            path=c.get("path") or "/", path_specified=True, secure=bool(c.get("secure")),
            expires=c.get("expires"), discard=c.get("expires") is None, comment=None,
            comment_url=None, rest={"HttpOnly": None} if c.get("http_only") else {},
        ))
    return jar


# D5: prefer AAC (141 then 140), Opus otherwise.
PREFERENCE = ["141", "140", "774", "251", "250", "249", "139"]


def pick(formats):
    audio = [f for f in formats if f.get("vcodec") == "none" and f.get("acodec") not in (None, "none") and f.get("url")]
    audio = [f for f in audio if not f.get("has_drm")]
    # Skip dynamic-range-compressed duplicates and non-original dubs.
    plain = [f for f in audio if "drc" not in str(f.get("format_id", "")) and not f.get("is_dash_periods")]
    pool = plain or audio
    orig = [f for f in pool if "original" in str(f.get("format_note", "")).lower() or f.get("language_preference", 0) >= 10]
    if orig:
        pool = orig
    by_id = {str(f.get("format_id", "")).split("-")[0]: f for f in pool}
    for itag in PREFERENCE:
        if itag in by_id:
            return by_id[itag]
    if pool:
        return max(pool, key=lambda f: f.get("abr") or f.get("tbr") or 0)
    return None


def resolve(req):
    cookies = req.get("cookies") or []
    logger = QuietLogger()
    opts = {
        "quiet": True,
        "no_warnings": True,
        "skip_download": True,
        "noplaylist": True,
        "logger": logger,
        "socket_timeout": 15,
        "extract_flat": False,
        "cachedir": req.get("cache_dir") or False,
    }
    rt = req.get("js_runtime")
    if rt and rt.get("name"):
        opts["js_runtimes"] = {rt["name"]: ({"path": rt["path"]} if rt.get("path") else {})}
    vid = req["video_id"]
    if not re.fullmatch(r"[A-Za-z0-9_-]{11}", vid):
        return {"ok": False, "error": "invalid video id", "kind": "unknown"}
    url = "https://music.youtube.com/watch?v=" + vid
    t0 = time.time()
    with yt_dlp.YoutubeDL(opts) as ydl:
        # Cookies live only in this in-memory jar for the duration of the call.
        ydl.cookiejar.clear()
        for c in make_jar(cookies):
            ydl.cookiejar.set_cookie(c)
        try:
            info = ydl.extract_info(url, download=False, process=False)
        except Exception as e:
            msg = scrub(e) or (logger.errors[-1] if logger.errors else "extraction failed")
            return {"ok": False, "error": msg, "kind": classify(msg)}
        finally:
            pass
        f = pick(info.get("formats") or [])
        ydl.cookiejar.clear()
    if not f:
        return {"ok": False, "error": "no audio formats for this video", "kind": "unavailable"}
    furl = f["url"]
    m = re.search(r"[?&]expire=(\d+)", furl)
    return {
        "ok": True,
        "format": {
            "url": furl,
            "headers": f.get("http_headers") or {},
            "itag": int(re.sub(r"\D.*$", "", str(f.get("format_id", "0"))) or 0),
            "ext": f.get("ext"),
            "acodec": f.get("acodec"),
            "abr": f.get("abr") or f.get("tbr"),
            "filesize": f.get("filesize") or f.get("filesize_approx"),
            "duration": info.get("duration"),
            "expires": int(m.group(1)) if m else None,
            # Some clients' URLs only work after this time (yt-dlp waits too).
            "available_at": f.get("available_at"),
            "authenticated": bool(cookies),
            "elapsed": round(time.time() - t0, 3),
        },
    }


def main():
    out = sys.stdout
    out.write(json.dumps({"id": 0, "ok": True, "hello": {"protocol": PROTOCOL, "yt_dlp": yt_dlp.version.__version__}}) + "\n")
    out.flush()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        if len(line) > 1_048_576:
            out.write(json.dumps({"id": 0, "ok": False, "error": "request too large", "kind": "unknown"}) + "\n")
            out.flush()
            continue
        try:
            req = json.loads(line)
        except Exception:
            continue
        rid = req.get("id", 0)
        op = req.get("op")
        try:
            if op == "health":
                res = {"ok": True, "health": {"yt_dlp": yt_dlp.version.__version__}}
            elif op == "resolve":
                res = resolve(req)
            elif op == "shutdown":
                break
            else:
                res = {"ok": False, "error": "unknown op", "kind": "unknown"}
        except Exception as e:
            res = {"ok": False, "error": scrub("%s: %s" % (type(e).__name__, e)), "kind": "unknown"}
            sys.stderr.write(scrub(traceback.format_exc()) + "\n")
        res["id"] = rid
        out.write(json.dumps(res) + "\n")
        out.flush()


if __name__ == "__main__":
    main()

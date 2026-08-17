#!/usr/bin/env python3
"""Static dev server with aggressive no-cache headers, so a reverse proxy / CDN
in front of the container never serves stale index.html / .js / .wasm.

Usage: python3 serve.py [port]        (default 8080)

Also exposes the level editor's persistence API (used by tools/levels.html):

    PUT /levels/<file>.json            write a floor / index file
    PUT /levels/<subdir>/<file>.json   (one optional sub-directory, e.g. samples/)
    PUT /props/props.json              the prop library's pixel-art settings
                                       (saved by the ?viz PROPS page)

  * Only paths under levels/ — plus exactly props/props.json — are writable;
    segments must match [A-Za-z0-9_-]+ and the file must end in .json (no
    "..", no absolute paths).
  * The body must be valid JSON (it is written verbatim, so the editor's canonical
    formatting is preserved byte-for-byte).
  * After writing a floor file, index.json in the same directory is updated
    (entry {id, file, name}, kept sorted by id). Writing index.json itself only
    validates + writes it.
  * Response: 200 {"ok": true, "path": ..., "index": ...} or 4xx {"ok": false, "error": ...}
"""
import http.server, socketserver, os, sys, json, re

ROOT = os.path.dirname(os.path.abspath(__file__))
os.chdir(ROOT)
LEVELS_DIR = os.path.join(ROOT, "levels")
SEG_RE = re.compile(r"^[A-Za-z0-9_-]+$")
FILE_RE = re.compile(r"^[A-Za-z0-9_-]+\.json$")
MAX_BODY = 5 * 1024 * 1024
# Cap on how many floor files may live under levels/ (incl. subdirs) —
# a cheap guard against someone spamming new files through the API.
MAX_LEVEL_FILES = 200

# Write access (PUT/DELETE) requires a shared secret sent as the
# `X-Editor-Token` header. The token comes from $EDITOR_TOKEN, else from the
# file .editor-token next to this script (created with a random value on first
# start and printed once so you can paste it into the editor prompt).
TOKEN_FILE = os.path.join(ROOT, ".editor-token")


def editor_token():
    tok = os.environ.get("EDITOR_TOKEN", "").strip()
    if tok:
        return tok
    try:
        with open(TOKEN_FILE, "r", encoding="utf-8") as fh:
            tok = fh.read().strip()
        if tok:
            return tok
    except FileNotFoundError:
        pass
    import secrets
    tok = secrets.token_urlsafe(24)
    with open(TOKEN_FILE, "w", encoding="utf-8") as fh:
        fh.write(tok + "\n")
    try:
        os.chmod(TOKEN_FILE, 0o600)
    except OSError:
        pass
    return tok


def count_level_files():
    n = 0
    for _root, _dirs, files in os.walk(os.path.join(ROOT, "levels")):
        n += sum(1 for f in files if f.endswith(".json"))
    return n


def safe_levels_path(url_path):
    """Map a request path to an absolute path under levels/, or None."""
    from urllib.parse import urlparse, unquote
    p = unquote(urlparse(url_path).path)
    prefix = "/levels/"
    if not p.startswith(prefix):
        return None
    rel = p[len(prefix):]
    parts = rel.split("/")
    if len(parts) < 1 or len(parts) > 2:
        return None
    for seg in parts[:-1]:
        if not SEG_RE.match(seg):
            return None
    if not FILE_RE.match(parts[-1]):
        return None
    abs_path = os.path.realpath(os.path.join(LEVELS_DIR, *parts))
    if os.path.commonpath([abs_path, os.path.realpath(LEVELS_DIR)]) != os.path.realpath(LEVELS_DIR):
        return None
    return abs_path


# The prop settings document (see docs/PROPS_FORMAT.md): the only writable
# path outside levels/.
PROPS_JSON = os.path.join(ROOT, "props", "props.json")


def safe_props_path(url_path):
    """Map a request path to props/props.json, or None."""
    from urllib.parse import urlparse, unquote
    p = unquote(urlparse(url_path).path)
    return PROPS_JSON if p == "/props/props.json" else None


def validate_props_doc(doc):
    """Shape check of a props.json body (tools/gen_props.py is the full
    validator): {"props": [{"kind": str, "px": 1..10, "layers": [{"name", "pixel"}]}]}."""
    if not isinstance(doc, dict) or not isinstance(doc.get("props"), list):
        return 'top level must be {"props": [...]}'
    for i, p in enumerate(doc["props"]):
        if not isinstance(p, dict) or not isinstance(p.get("kind"), str):
            return "props[%d]: needs a string kind" % i
        px = p.get("px", 1)
        if not isinstance(px, int) or isinstance(px, bool) or not 1 <= px <= 10:
            return "%s: px must be an integer 1..10" % p["kind"]
        for l in p.get("layers", []):
            if not isinstance(l, dict) or not isinstance(l.get("name"), str):
                return "%s: layers[] entries need a name" % p["kind"]
            if l.get("pixel", "before") not in ("before", "after"):
                return "%s/%s: pixel must be 'before' or 'after'" % (p["kind"], l["name"])
    return None


def index_entries(idx):
    """Return (container, list) for the supported index.json shapes."""
    if isinstance(idx, dict) and isinstance(idx.get("floors"), list):
        return idx, idx["floors"]
    if isinstance(idx, list):
        return idx, idx
    return None, None


def entry_file(e):
    if isinstance(e, str):
        return e
    if isinstance(e, dict):
        return e.get("file")
    return None


def update_index(dir_path, file_name, floor):
    """Insert/replace `file_name` in dir_path/index.json, sorted by id."""
    idx_path = os.path.join(dir_path, "index.json")
    idx = {"floors": []}
    if os.path.exists(idx_path):
        try:
            with open(idx_path, "r", encoding="utf-8") as fh:
                idx = json.load(fh)
        except Exception:
            idx = {"floors": []}
    container, entries = index_entries(idx)
    if entries is None:
        idx = {"floors": []}
        container, entries = idx, idx["floors"]
    fid = floor.get("id") if isinstance(floor, dict) else None
    fname = floor.get("name") if isinstance(floor, dict) else None
    string_style = all(isinstance(e, str) for e in entries) and len(entries) > 0
    new_entry = file_name if string_style else {"id": fid, "file": file_name, "name": fname}
    entries[:] = [e for e in entries if entry_file(e) != file_name]
    entries.append(new_entry)

    def sort_key(e):
        if isinstance(e, dict) and isinstance(e.get("id"), (int, float)):
            return (0, e["id"], entry_file(e) or "")
        return (1, 0, entry_file(e) or "")

    entries.sort(key=sort_key)
    write_index(idx_path, idx)
    return idx


def dump_index(idx):
    """Compact-per-entry style, matching the checked-in levels/index.json:
    {"floors": [ one {"id","file","name"} object per line ]}."""
    if isinstance(idx, dict) and isinstance(idx.get("floors"), list) and len(idx) == 1:
        lines = ["    " + json.dumps(e, ensure_ascii=False) for e in idx["floors"]]
        return "{\n  \"floors\": [\n" + ",\n".join(lines) + "\n  ]\n}\n"
    return json.dumps(idx, indent=2, ensure_ascii=False) + "\n"


def write_index(idx_path, idx):
    tmp = idx_path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        fh.write(dump_index(idx))
    os.replace(tmp, idx_path)


# Paths that must never be served: dotfiles/dirs (.editor-token, .git, .claude…),
# local-only reference audio, build output and session screenshots.
PRIVATE_TOP = {"inspirations", ".audio-ref", "target", "shots", "node_modules"}


def is_private_path(url_path):
    parts = [p for p in url_path.split("?", 1)[0].split("/") if p]
    if any(p.startswith(".") for p in parts):
        return True
    return bool(parts) and parts[0] in PRIVATE_TOP


class NoCache(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        if is_private_path(self.path):
            return self.send_error(404, "Not found")
        return super().do_GET()

    def do_HEAD(self):
        if is_private_path(self.path):
            return self.send_error(404, "Not found")
        return super().do_HEAD()

    def end_headers(self):
        self.send_header("Cache-Control", "no-store, no-cache, must-revalidate, max-age=0")
        self.send_header("Pragma", "no-cache")
        self.send_header("Expires", "0")
        super().end_headers()

    def _json(self, code, obj):
        data = (json.dumps(obj) + "\n").encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _authorized(self):
        import hmac
        sent = self.headers.get("X-Editor-Token", "")
        return bool(sent) and hmac.compare_digest(sent, editor_token())

    def do_PUT(self):
        if not self._authorized():
            return self._json(401, {"ok": False, "error": "missing or wrong X-Editor-Token"})
        props_path = safe_props_path(self.path)
        abs_path = props_path or safe_levels_path(self.path)
        if abs_path is None:
            return self._json(403, {"ok": False, "error": "PUT allowed only for levels/**/*.json and props/props.json"})
        if props_path is None and not os.path.exists(abs_path) and count_level_files() >= MAX_LEVEL_FILES:
            return self._json(429, {"ok": False, "error": "too many level files"})
        try:
            length = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            length = 0
        if length <= 0 or length > MAX_BODY:
            return self._json(400, {"ok": False, "error": "bad Content-Length"})
        body = self.rfile.read(length)
        try:
            doc = json.loads(body.decode("utf-8"))
        except Exception as e:
            return self._json(400, {"ok": False, "error": "body is not valid JSON: %s" % e})
        if props_path is not None:
            err = validate_props_doc(doc)
            if err:
                return self._json(400, {"ok": False, "error": "props.json: " + err})
        os.makedirs(os.path.dirname(abs_path), exist_ok=True)
        tmp = abs_path + ".tmp"
        with open(tmp, "wb") as fh:
            fh.write(body)
        os.replace(tmp, abs_path)
        rel = os.path.relpath(abs_path, ROOT)
        idx = None
        if props_path is None and os.path.basename(abs_path) != "index.json":
            idx = update_index(os.path.dirname(abs_path), os.path.basename(abs_path), doc)
        return self._json(200, {"ok": True, "path": rel, "bytes": len(body), "index": idx})

    def do_DELETE(self):
        if not self._authorized():
            return self._json(401, {"ok": False, "error": "missing or wrong X-Editor-Token"})
        abs_path = safe_levels_path(self.path)
        if abs_path is None or os.path.basename(abs_path) == "index.json":
            return self._json(403, {"ok": False, "error": "DELETE allowed only for levels/**/floor json"})
        if not os.path.exists(abs_path):
            return self._json(404, {"ok": False, "error": "not found"})
        os.remove(abs_path)
        # drop it from index.json in the same directory
        idx_path = os.path.join(os.path.dirname(abs_path), "index.json")
        idx = None
        if os.path.exists(idx_path):
            try:
                with open(idx_path, "r", encoding="utf-8") as fh:
                    idx = json.load(fh)
                container, entries = index_entries(idx)
                if entries is not None:
                    entries[:] = [e for e in entries if entry_file(e) != os.path.basename(abs_path)]
                    write_index(idx_path, idx)
            except Exception:
                idx = None
        return self._json(200, {"ok": True, "path": os.path.relpath(abs_path, ROOT), "index": idx})


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8080
    print("editor write token (X-Editor-Token): %s" % editor_token(), flush=True)
    socketserver.TCPServer.allow_reuse_address = True
    with socketserver.TCPServer(("0.0.0.0", port), NoCache) as httpd:
        print("serving :%d (no-store) — PUT/DELETE enabled under /levels/, PUT /props/props.json" % port)
        httpd.serve_forever()

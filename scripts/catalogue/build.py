#!/usr/bin/env python3
"""Orvo Studio plugin catalogue.

Runs daily on GitHub Actions and publishes one file, plugins.json (plus icons), to the
`catalogue` branch. Orvo Studio's plugin store reads that file.

Sources, merged by plugin ID:
  openaction  – OpenDeck's catalogue (openactionapi.github.io)
  community   – the curated list at github.com/madmatt/streamdeck-plugins
  github      – GitHub repos tagged streamdeck-plugin / stream-deck-plugin
  barraider   – BarRaider's Stream Deck plugins
  archive     – the old Elgato App Store archive (plugins.amankhanna.me), only for
                plugins no other source has a newer copy of

For every plugin the real manifest.json inside its release file is read (only the bytes
needed, using HTTP range requests) to learn which systems it supports. Plugins whose
manifest is encrypted (Elgato Marketplace builds) can't run in Orvo and are left out.
Results are cached between runs, so unchanged plugins cost no downloads.
"""

import io
import json
import os
import re
import sys
import time
import zipfile
from datetime import datetime, timezone

import requests

OUT = sys.argv[1] if len(sys.argv) > 1 else "catalogue"
ICONS = os.path.join(OUT, "icons")
ICON_BASE = "https://raw.githubusercontent.com/Som3Rand0m3Dud3/Orvo-Deck/catalogue/icons/"
TOKEN = os.environ.get("GITHUB_TOKEN", "")
MAX_NEW_ARCHIVE = int(os.environ.get("MAX_NEW_ARCHIVE", "200"))  # archive files read per run
os.makedirs(ICONS, exist_ok=True)

S = requests.Session()
S.headers["User-Agent"] = "orvo-studio-catalogue"
GH = requests.Session()
GH.headers.update({"Accept": "application/vnd.github+json", "User-Agent": "orvo-studio-catalogue"})
if TOKEN:
    GH.headers["Authorization"] = f"Bearer {TOKEN}"

errors = []


def log(msg):
    print(msg, flush=True)


def gh(path, etag=None):
    """GitHub API GET. Returns (status, json, etag). 304 when unchanged (free)."""
    headers = {"If-None-Match": etag} if etag else {}
    for attempt in range(3):
        r = GH.get("https://api.github.com" + path, headers=headers, timeout=30)
        if r.status_code == 403 and r.headers.get("X-RateLimit-Remaining") == "0":
            wait = max(5, int(r.headers.get("X-RateLimit-Reset", time.time() + 60)) - int(time.time()) + 2)
            if wait > 900:
                raise RuntimeError("rate limited")
            log(f"rate limited, waiting {wait}s")
            time.sleep(wait)
            continue
        if r.status_code in (502, 503):
            time.sleep(3)
            continue
        if r.status_code == 304:
            return 304, None, etag
        try:
            body = r.json()
        except ValueError:
            body = None
        return r.status_code, body, r.headers.get("ETag")
    return 0, None, None


# ---------------------------------------------------------------- previous run (cache)
prev = {}
try:
    with open(os.path.join(OUT, "cache.json")) as f:
        prev = json.load(f)
except (OSError, ValueError):
    pass
repo_cache = prev.get("repos", {})  # "owner/repo" -> {etag, asset, result}
asset_cache = prev.get("assets", {})  # download url -> result (or {"skip": reason})
# Results from an older reader lack newer fields (language, better icons): re-read those.
READER_VERSION = 2
def fresh(r):
    return isinstance(r, dict) and ("skip" in r or r.get("rv", 0) >= READER_VERSION)
# Retry anything that failed for a reason other than encryption (the reader may have improved).
asset_cache = {u: r for u, r in asset_cache.items() if "skip" not in r or str(r["skip"]).startswith("encrypted")}
repo_cache = {k: v for k, v in repo_cache.items() if not (isinstance(v.get("result"), dict) and "skip" in v["result"])}
old_asset_cache = asset_cache
asset_cache = {u: r for u, r in asset_cache.items() if fresh(r)}
new_repo_cache = {}

# ---------------------------------------------------------------- candidates by repo
repos = {}  # "owner/repo" lower -> info
RANK = {"openaction": 0, "community": 1, "barraider": 2, "github": 3}


def add_repo(full, source, **info):
    if not full or "/" not in full:
        return
    key = full.strip("/").lower()
    have = repos.get(key)
    if have and RANK[have["source"]] <= RANK[source]:
        for k, v in info.items():  # fill gaps only
            have.setdefault(k, v)
        return
    merged = dict(have or {})
    merged.update({k: v for k, v in info.items() if v not in (None, "")})
    merged.update({"full": full.strip("/"), "source": source})
    repos[key] = merged


def repo_of(url):
    m = re.match(r"https?://(?:www\.)?github\.com/([^/\s]+)/([^/\s#?]+)", url or "")
    return f"{m.group(1)}/{m.group(2).removesuffix('.git')}" if m else None


# 1. OpenDeck / OpenAction catalogue
openaction = {}
try:
    openaction = S.get("https://openactionapi.github.io/plugins/catalogue.json", timeout=30).json()
except Exception as e:  # noqa: BLE001
    errors.append(f"openaction: {e}")
direct_openaction = []
for pid, p in openaction.items():
    full = repo_of(p.get("repository"))
    info = dict(oa_id=pid, name=p.get("name"), author=p.get("author"), description=p.get("description"),
                icon=f"https://openactionapi.github.io/plugins/icons/{pid}.png", flags=p.get("flags"))
    if p.get("download_url"):
        direct_openaction.append((pid, p, full, info))
    elif full:
        add_repo(full, "openaction", **info)
log(f"openaction: {len(openaction)}")

# 2. Community list
try:
    md = S.get("https://raw.githubusercontent.com/madmatt/streamdeck-plugins/master/index.md", timeout=30).text
    n = 0
    for line in md.splitlines():
        cells = [c.strip() for c in line.split("|")]
        if len(cells) < 6 or cells[0] in ("Plugin Name", "") or set(cells[0]) <= set("-: "):
            continue
        links = re.findall(r"\((https?://[^)]+)\)", cells[-1])
        full = repo_of(links[0]) if links else None
        author = re.sub(r"\[([^\]]+)\].*", r"\1", cells[4]).strip("[] ")
        hint = [os_ for os_, c in (("windows", cells[2]), ("mac", cells[3])) if "✔" in c]
        if full:
            add_repo(full, "community", name=cells[0], description=cells[1], author=author, hint=hint)
            n += 1
    log(f"community: {n}")
except Exception as e:  # noqa: BLE001
    errors.append(f"community: {e}")

# 3. GitHub topics
for topic in ("streamdeck-plugin", "stream-deck-plugin", "streamdeck-plugins"):
    for page in range(1, 11):
        st, body, _ = gh(f"/search/repositories?q=topic:{topic}+archived:false+fork:false&sort=stars&per_page=100&page={page}")
        if st != 200 or not body or not body.get("items"):
            break
        for r in body["items"]:
            add_repo(r["full_name"], "github", name=r["name"], description=r.get("description"),
                     author=r["owner"]["login"], stars=r.get("stargazers_count", 0))
        if len(body["items"]) < 100:
            break
        time.sleep(2)  # search API: 30 requests a minute
log(f"after github topics: {len(repos)} repos")

# 4. BarRaider
for page in range(1, 6):
    st, body, _ = gh(f"/users/BarRaider/repos?per_page=100&page={page}")
    if st != 200 or not body:
        break
    for r in body:
        if r["name"].lower().startswith("streamdeck-") and not r.get("archived") and not r.get("fork"):
            add_repo(r["full_name"], "barraider", name=r["name"], description=r.get("description"), author="BarRaider",
                     stars=r.get("stargazers_count", 0))
    if len(body) < 100:
        break
log(f"total repos: {len(repos)}")


# ---------------------------------------------------------------- reading a plugin file
class HttpFile(io.RawIOBase):
    """A seekable remote file read with HTTP range requests, so a zip's directory, manifest and
    icon can be read without downloading the whole plugin. Reads are cached in 64 KB blocks."""

    BLOCK = 64 * 1024

    def __init__(self, url):
        r = S.get(url, headers={"Range": "bytes=0-0"}, timeout=60, allow_redirects=True)
        if r.status_code != 206 or "Content-Range" not in r.headers:
            raise IOError(f"no range support ({r.status_code})")
        self.url = r.url
        self.size = int(r.headers["Content-Range"].split("/")[-1])
        self.pos = 0
        self.blocks = {}

    def readable(self):
        return True

    def seekable(self):
        return True

    def tell(self):
        return self.pos

    def seek(self, offset, whence=0):
        self.pos = offset if whence == 0 else self.pos + offset if whence == 1 else self.size + offset
        return self.pos

    def _block(self, n):
        if n not in self.blocks:
            start = n * self.BLOCK
            end = min(start + self.BLOCK, self.size) - 1
            r = S.get(self.url, headers={"Range": f"bytes={start}-{end}"}, timeout=60)
            if r.status_code != 206:
                raise IOError(f"range read failed ({r.status_code})")
            self.blocks[n] = r.content
        return self.blocks[n]

    def read(self, size=-1):
        if size is None or size < 0:
            size = self.size - self.pos
        size = max(0, min(size, self.size - self.pos))
        out = bytearray()
        while size > 0:
            n, off = divmod(self.pos, self.BLOCK)
            chunk = self._block(n)[off : off + size]
            if not chunk:
                break
            out += chunk
            self.pos += len(chunk)
            size -= len(chunk)
        return bytes(out)

    def readinto(self, b):
        data = self.read(len(b))
        b[: len(data)] = data
        return len(data)



def platforms_of(manifest):
    found = set()
    for o in manifest.get("OS") or []:
        p = str((o or {}).get("Platform", "")).lower()
        if p.startswith("win"):
            found.add("windows")
        elif p.startswith("mac") or p == "darwin":
            found.add("mac")
        elif p.startswith("lin"):
            found.add("linux")
    code = {k.lower(): v for k, v in manifest.items()}
    if code.get("codepathwin"):
        found.add("windows")
    if code.get("codepathmac"):
        found.add("mac")
    if code.get("codepathlin"):
        found.add("linux")
    for triple in (manifest.get("CodePaths") or {}):
        t = triple.lower()
        found.add("windows" if "windows" in t else "mac" if "apple" in t or "darwin" in t else "linux" if "linux" in t else "")
    found.discard("")
    # A plain "CodePath" that is JavaScript/HTML runs everywhere Node or a web view does.
    cp = str(code.get("codepath", ""))
    if not found and cp.endswith((".js", ".mjs", ".cjs", ".html", ".htm")):
        found.update({"windows", "mac", "linux"})
    return sorted(found) or None


# ---------------------------------------------------------------- language
try:
    from lingua import Language, LanguageDetectorBuilder
    _LANGS = [Language.ENGLISH, Language.SPANISH, Language.GERMAN, Language.FRENCH, Language.PORTUGUESE,
              Language.ITALIAN, Language.DUTCH, Language.POLISH, Language.RUSSIAN, Language.UKRAINIAN,
              Language.CZECH, Language.SWEDISH, Language.TURKISH, Language.ARABIC, Language.CHINESE,
              Language.JAPANESE, Language.KOREAN]
    DETECTOR = LanguageDetectorBuilder.from_languages(*_LANGS).with_minimum_relative_distance(0.15).build()
except ImportError:  # language tags are optional
    DETECTOR = None


def visible_text(html):
    html = re.sub(r"(?is)<(script|style)[^>]*>.*?</\1>", " ", html)
    attrs = " ".join(re.findall(r'(?i)(?:placeholder|title|label)\s*=\s*"([^"]{2,80})"', html))
    text = re.sub(r"(?s)<[^>]+>", " ", html)
    text = re.sub(r"&[a-z#0-9]+;", " ", text)
    return re.sub(r"\s+", " ", text + " " + attrs).strip()


def language_of(text):
    """ISO 639-1 code of the text, or None when unsure or too short."""
    if not DETECTOR:
        return None
    words = re.findall(r"[^\W\d_]{2,}", text)
    if len(words) < 4:
        return None
    lang = DETECTOR.detect_language_of(" ".join(words)[:4000])
    return lang.iso_code_639_1.name.lower() if lang else None


def read_plugin(url, allow_full=False):
    """Returns {id, name, version, platforms, icon} or {"skip": reason}."""
    zf = None
    try:
        zf = zipfile.ZipFile(io.BufferedReader(HttpFile(url), buffer_size=HttpFile.BLOCK))
    except Exception as e:  # noqa: BLE001
        if not allow_full:
            return {"skip": f"no range: {e}"[:200]}
        try:
            data = S.get(url, timeout=180).content
            zf = zipfile.ZipFile(io.BytesIO(data))
        except Exception as e2:  # noqa: BLE001
            return {"skip": f"download: {e2}"[:200]}
    try:
        # Zips made on Windows may use "\\" in paths: match on "/" but read with the real name.
        real = {n.replace("\\", "/"): n for n in zf.namelist()}
        names = list(real)
        inner = [n for n in names if n.lower().endswith((".streamdeckplugin",))]
        if inner and not any(n.lower().endswith("manifest.json") for n in names):
            zf = zipfile.ZipFile(io.BytesIO(zf.read(real[inner[0]])))
            real = {n.replace("\\", "/"): n for n in zf.namelist()}
            names = list(real)
        manifests = sorted((n for n in names if re.search(r"[^/]+\.sdplugin/manifest\.json$", n, re.I)), key=len)
        if not manifests:
            return {"skip": "no manifest"}
        mpath = manifests[0]
        raw = zf.read(real[mpath])
        if raw.startswith(b"ELGATO"):
            return {"skip": "encrypted (Elgato Marketplace build)"}
        manifest = json.loads(raw.decode("utf-8-sig"))
        folder = mpath[: -len("/manifest.json")]
        pid = re.sub(r"\.sdplugin$", "", folder.split("/")[-1], flags=re.I)
        result = {"id": pid, "name": manifest.get("Name"), "version": manifest.get("Version"),
                  "platforms": platforms_of(manifest), "category": manifest.get("Category")}
        lower = {n.lower(): n for n in names}

        def find(rel):
            rel = rel.replace("\\", "/").lstrip("./") if rel else ""
            return lower.get(f"{folder}/{rel}".lower()) if rel else None

        # Icon: the plugin icon, else the category icon, else the first action's icon.
        icons = [manifest.get("Icon"), manifest.get("CategoryIcon")]
        icons += [a.get("Icon") for a in manifest.get("Actions") or [] if isinstance(a, dict)]
        for icon in filter(None, icons):
            stem = re.sub(r"\.(png|svg|jpe?g|gif)$", "", icon, flags=re.I)
            hit = next(filter(None, (find(c) for c in (f"{stem}@2x.png", f"{stem}.png", icon, f"{stem}.svg"))), None)
            if not hit:
                continue
            try:
                data = zf.read(real[hit])
                if hit.lower().endswith(".svg"):
                    # GitHub's raw server sends SVGs as plain text, which images can't show: convert.
                    import cairosvg
                    data = cairosvg.svg2png(bytestring=data, output_width=144, output_height=144)
                if len(data) < 400_000:
                    fname = re.sub(r"[^A-Za-z0-9._-]", "_", pid) + ".png"
                    with open(os.path.join(ICONS, fname), "wb") as f:
                        f.write(data)
                    result["icon"] = ICON_BASE + fname
                    break
            except Exception:  # noqa: BLE001
                pass

        # Language: what the user actually sees (action names, tooltips, settings panels).
        actions = [a for a in manifest.get("Actions") or [] if isinstance(a, dict)]
        seen = " ".join(str(a.get(k) or "") for a in actions for k in ("Name", "Tooltip"))
        panels = {manifest.get("PropertyInspectorPath")} | {a.get("PropertyInspectorPath") for a in actions}
        for pi in [p for p in panels if p][:3]:
            hit = find(pi)
            if hit and zf.getinfo(real[hit]).file_size < 300_000:
                try:
                    seen += " " + visible_text(zf.read(real[hit]).decode("utf-8", "ignore"))
                except Exception:  # noqa: BLE001
                    pass
        lang = language_of(seen) or language_of(seen + " " + str(manifest.get("Description") or ""))
        has_en = bool(find("en.json"))
        if has_en:
            lang = "en"  # Orvo shows the plugin's own English translation
        if lang:
            result["language"] = lang
        result["rv"] = READER_VERSION
        return result
    except Exception as e:  # noqa: BLE001
        return {"skip": f"read: {e}"[:200]}


def pick_asset(assets):
    good = [a for a in assets if a["name"].lower().endswith(".streamdeckplugin")]
    good = good or [a for a in assets if a["name"].lower().endswith(".zip") and "source" not in a["name"].lower()]
    if not good:
        return None
    # prefer a Windows build when a plugin ships one file per system
    good.sort(key=lambda a: (0 if "win" in a["name"].lower() else 1, len(a["name"])))
    return good[0]


# ---------------------------------------------------------------- resolve repos
plugins = {}  # id -> entry
used_api = 0


def by_words(entry):
    """A plugin that says it is for one system (e.g. "… on Linux") is taken at its word."""
    text = f"{entry.get('name') or ''} {entry.get('description') or ''}".lower()
    says = [os_ for os_, pat in (("linux", r"\blinux\b"), ("mac", r"\bmac ?os\b|\bmacos\b"), ("windows", r"\bwindows\b")) if re.search(pat, text)]
    if len(says) == 1 and says[0] != "windows":
        entry["platforms"], entry["platforms_known"] = says, True
    return entry


def put(entry):
    entry = by_words(entry)
    have = plugins.get(entry["id"])
    if have and (have.get("updated") or "") >= (entry.get("updated") or ""):
        return
    plugins[entry["id"]] = entry


for i, (key, info) in enumerate(sorted(repos.items())):
    cached = repo_cache.get(key, {})
    stale = isinstance(cached.get("result"), dict) and not fresh(cached["result"])
    st, rel, etag = gh(f"/repos/{info['full']}/releases/latest", None if stale else cached.get("etag"))
    used_api += 1
    if st == 304:
        new_repo_cache[key] = cached
        res = cached.get("result")
    elif st == 200 and rel:
        asset = pick_asset(rel.get("assets", []))
        if not asset:
            new_repo_cache[key] = {"etag": etag, "result": None}
            continue
        url = asset["browser_download_url"]
        res = asset_cache.get(url) or read_plugin(url)
        asset_cache[url] = res
        if "skip" not in res:
            res = dict(res, download_url=url, updated=rel.get("published_at"), release=rel.get("tag_name"))
        new_repo_cache[key] = {"etag": etag, "result": res}
    else:
        new_repo_cache[key] = {"result": None}
        continue
    if not res or "skip" in res:
        continue
    put({
        "id": info.get("oa_id") or res["id"],
        "name": info.get("name") if info["source"] in ("openaction", "community") else (res.get("name") or info.get("name")),
        "author": info.get("author"),
        "description": info.get("description"),
        "source": "open-source",
        "via": info["source"],
        "repository": f"https://github.com/{info['full']}",
        "download_url": res["download_url"],
        "version": res.get("version") or res.get("release"),
        "updated": res.get("updated"),
        "platforms": res.get("platforms") or info.get("hint") or None,
        "platforms_known": bool(res.get("platforms")),
        "icon": res.get("icon") or info.get("icon"),
        "language": res.get("language"),
        "flags": info.get("flags"),
        "stars": info.get("stars"),
    })
    if i % 50 == 0:
        log(f"  {i}/{len(repos)} repos, {len(plugins)} plugins")

for pid, p, full, info in direct_openaction:
    url = p["download_url"]
    res = asset_cache.get(url) or read_plugin(url, allow_full=True)
    asset_cache[url] = res
    if "skip" in res:
        continue
    put({"id": pid, "name": p.get("name"), "author": p.get("author"), "description": p.get("description"),
         "source": "open-source", "via": "openaction", "repository": p.get("repository") or "", "download_url": url,
         "version": res.get("version"), "updated": None, "platforms": res.get("platforms"),
         "platforms_known": bool(res.get("platforms")), "icon": res.get("icon") or info["icon"],
         "language": res.get("language"), "flags": p.get("flags")})

log(f"open-source plugins: {len(plugins)}")

# 5. Old Elgato App Store archive (static: results cached for good)
archive = []
try:
    archive = S.get("https://plugins.amankhanna.me/catalogue.json", timeout=60).json()
except Exception as e:  # noqa: BLE001
    errors.append(f"archive: {e}")
new_reads = 0
archived = 0
for a in archive:
    pid = a.get("id")
    if not pid or pid in plugins:
        continue  # a newer open-source copy exists
    url = f"https://plugins.amankhanna.me/rezipped/{pid}.zip"
    res = asset_cache.get(url)
    if res is None and new_reads < MAX_NEW_ARCHIVE:
        res = read_plugin(url, allow_full=True)
        asset_cache[url] = res
        new_reads += 1
    elif res is None and url in old_asset_cache:
        res = asset_cache[url] = old_asset_cache[url]  # older read: re-read on a later run
    if res and "skip" in res and res["skip"].startswith("encrypted"):
        continue
    known = bool(res and "skip" not in res and res.get("platforms"))
    plugins[pid] = by_words({
        "id": pid, "name": a.get("name"), "author": a.get("author"), "description": a.get("description"),
        "source": "archive", "via": "archive", "repository": "", "download_url": url,
        "version": (res or {}).get("version"), "updated": None,
        "platforms": (res or {}).get("platforms") if known else None, "platforms_known": known,
        "icon": (res or {}).get("icon") or f"https://plugins.amankhanna.me/icons/{pid}.png",
        "language": (res or {}).get("language"), "flags": None,
    })
    archived += 1
log(f"archive: {len(archive)} listed, {archived} added, {new_reads} read this run")

# ---------------------------------------------------------------- write
out = sorted(plugins.values(), key=lambda p: (p.get("name") or p["id"]).lower())
for p in out:
    for k in [k for k, v in p.items() if v in (None, "", [])]:
        del p[k]
with open(os.path.join(OUT, "plugins.json"), "w") as f:
    json.dump({"generated": datetime.now(timezone.utc).isoformat(timespec="seconds"), "count": len(out), "plugins": out},
              f, ensure_ascii=False, separators=(",", ":"))
with open(os.path.join(OUT, "cache.json"), "w") as f:
    json.dump({"repos": new_repo_cache, "assets": asset_cache}, f, separators=(",", ":"))
with open(os.path.join(OUT, "errors.json"), "w") as f:
    skipped = {u: r["skip"] for u, r in asset_cache.items() if isinstance(r, dict) and "skip" in r}
    json.dump({"errors": errors, "skipped": skipped}, f, indent=1)
log(f"wrote {len(out)} plugins ({sum(1 for p in out if p['source'] == 'archive')} archived); API calls ~{used_api}")

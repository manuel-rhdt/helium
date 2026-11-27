#!/usr/bin/env python3
"""Renders every gallery/<name>/index.html to a PDF and builds a static page that shows them.

    python3 gallery/build.py [--umbruch target/release/umbruch] [--out _site] [names...]

Each document is a directory with an `index.html`; its `<title>`, `<meta name="description">`
and `<meta name="keywords">` describe it on the page. `<meta name="gallery-thumbnail"
content="3">` picks the page of the thumbnail (the first by default), and `<meta
name="gallery-fonts" content="system">` renders with the fonts installed on the machine
instead of only the fonts built into Umbruch. The PDF is rendered with the directory of
the document as working directory, so relative references (stylesheets, images) resolve.
Needs `pdftoppm` and `pdfinfo` (poppler-utils) for the thumbnails and page counts.
"""
import argparse
import html
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = "https://github.com/manuel-rhdt/umbruch"


def meta(source: str, name: str) -> str:
    m = re.search(r'<meta\s+name="%s"\s+content="([^"]*)"' % name, source)
    return html.unescape(m.group(1)) if m else ""


def title(source: str) -> str:
    m = re.search(r"<title>(.*?)</title>", source, re.S)
    return html.unescape(m.group(1).strip()) if m else ""


def pages(pdf: Path) -> int:
    out = subprocess.run(["pdfinfo", str(pdf)], capture_output=True, text=True).stdout
    m = re.search(r"^Pages:\s+(\d+)", out, re.M)
    return int(m.group(1)) if m else 0


def render(umbruch: Path, doc: Path, out: Path, source: str) -> tuple[bool, str, float]:
    started = time.time()
    # The built-in fonts make the output the same on every machine. A document that needs fonts
    # for other scripts asks for the installed ones with <meta name="gallery-fonts" content="system">.
    flags = [] if meta(source, "gallery-fonts") == "system" else ["--no-system-fonts"]
    result = subprocess.run(
        [str(umbruch.resolve()), *flags, "index.html", str(out.resolve())],
        cwd=doc, capture_output=True, text=True, env={**os.environ, "RUST_LOG": "warn"},
    )
    log = (result.stdout + result.stderr).strip()
    return result.returncode == 0 and out.exists() and out.stat().st_size > 0, log, time.time() - started


def thumbnail(pdf: Path, png: Path, page: int = 1) -> bool:
    stem = png.with_suffix("")
    result = subprocess.run(
        ["pdftoppm", "-png", "-r", "100", "-f", str(page), "-l", str(page), "-singlefile", str(pdf), str(stem)],
        capture_output=True,
    )
    return result.returncode == 0


PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Umbruch gallery</title>
<style>
:root {{ --bg:#f5f5f2; --fg:#1b1b1f; --muted:#62626b; --card:#ffffff; --line:#e2e2dc; --accent:#b4540a; }}
@media (prefers-color-scheme: dark) {{
  :root {{ --bg:#141416; --fg:#ecece8; --muted:#9a9aa3; --card:#1d1d21; --line:#2c2c32; --accent:#f0a35e; }}
}}
* {{ box-sizing: border-box; }}
body {{ margin:0; background:var(--bg); color:var(--fg); font:16px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }}
main {{ max-width:1120px; margin:0 auto; padding:0 20px 48px; }}
header {{ padding:56px 0 28px; }}
h1 {{ margin:0 0 8px; font-size:clamp(30px,5vw,44px); letter-spacing:-.02em; }}
h1 span {{ color:var(--accent); }}
header p {{ margin:0; max-width:640px; color:var(--muted); font-size:18px; }}
.grid {{ display:grid; grid-template-columns:repeat(auto-fill,minmax(290px,1fr)); gap:22px; }}
.card {{ display:flex; flex-direction:column; background:var(--card); border:1px solid var(--line); border-radius:14px; overflow:hidden; }}
.thumb {{ display:block; background:#d9d9d3; aspect-ratio:4/3; overflow:hidden; }}
.thumb img {{ display:block; width:100%; height:100%; object-fit:cover; object-position:top center; transition:transform .25s; }}
.card:hover .thumb img {{ transform:scale(1.03); }}
.body {{ padding:16px 18px 18px; display:flex; flex-direction:column; gap:8px; flex:1; }}
.body h2 {{ margin:0; font-size:19px; line-height:1.25; }}
.body h2 a {{ color:inherit; text-decoration:none; }}
.body p {{ margin:0; color:var(--muted); font-size:14.5px; flex:1; }}
.tags {{ display:flex; flex-wrap:wrap; gap:6px; margin:2px 0 4px; }}
.tags span {{ font-size:12px; padding:2px 9px; border-radius:99px; border:1px solid var(--line); color:var(--muted); }}
.links {{ display:flex; gap:16px; align-items:center; font-size:14px; }}
.links a {{ color:var(--accent); text-decoration:none; font-weight:600; }}
.links a:hover {{ text-decoration:underline; }}
.links .n {{ margin-left:auto; color:var(--muted); font-size:13px; }}
.failed {{ border-color:#c0392b; }}
footer {{ margin-top:40px; color:var(--muted); font-size:13px; }}
footer a {{ color:inherit; }}
</style>
</head>
<body>
<main>
<header>
<h1>Umbruch <span>gallery</span></h1>
<p>Documents written in plain HTML and CSS, rendered to PDF by <a href="{repo}" style="color:inherit">Umbruch</a> on every push. Open a card to read the PDF, or look at its source.</p>
</header>
<section class="grid">
{cards}
</section>
<footer>Built from commit <a href="{repo}/commit/{sha}">{short}</a> · {date} · {count} documents</footer>
</main>
</body>
</html>
"""

CARD = """<article class="card{cls}">
<a class="thumb" href="{slug}.pdf">{thumb}</a>
<div class="body">
<h2><a href="{slug}.pdf">{title}</a></h2>
<div class="tags">{tags}</div>
<p>{description}</p>
<div class="links"><a href="{slug}.pdf">Open PDF</a><a href="{repo}/tree/master/gallery/{slug}">Source</a><span class="n">{pages}</span></div>
</div>
</article>"""


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--umbruch", default="target/release/umbruch")
    parser.add_argument("--out", default="_site")
    parser.add_argument("names", nargs="*")
    args = parser.parse_args()

    umbruch = Path(args.umbruch)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    docs = sorted(p.parent for p in ROOT.glob("*/index.html"))
    if args.names:
        docs = [d for d in docs if d.name in args.names]
    # an optional `order` file lists the slugs in the order of the page
    order_file = ROOT / "order.txt"
    if order_file.exists() and not args.names:
        order = [l.strip() for l in order_file.read_text().split() if l.strip()]
        docs.sort(key=lambda d: order.index(d.name) if d.name in order else len(order))

    cards, failures = [], []
    for doc in docs:
        slug = doc.name
        source = (doc / "index.html").read_text(encoding="utf-8")
        pdf = out / f"{slug}.pdf"
        ok, log, seconds = render(umbruch, doc, pdf, source)
        warnings = [l for l in log.splitlines() if "WARN" in l or "ERROR" in l]
        print(f"{'ok  ' if ok else 'FAIL'} {slug} ({seconds:.1f}s, {len(warnings)} warnings)", flush=True)
        if not ok:
            failures.append(slug)
            print(log, file=sys.stderr)
        thumb = ""
        count = ""
        if ok:
            png = out / f"{slug}.png"
            if thumbnail(pdf, png, int(meta(source, "gallery-thumbnail") or 1)):
                thumb = f'<img src="{slug}.png" alt="First page of {html.escape(title(source))}" loading="lazy">'
            n = pages(pdf)
            count = f"{n} page{'s' if n != 1 else ''} · {pdf.stat().st_size // 1024} KB"
        tags = "".join(f"<span>{html.escape(t.strip())}</span>" for t in meta(source, "keywords").split(",") if t.strip())
        cards.append(CARD.format(
            cls="" if ok else " failed", slug=slug, thumb=thumb, repo=REPO,
            title=html.escape(title(source) or slug), tags=tags,
            description=html.escape(meta(source, "description")),
            pages=count if ok else "render failed",
        ))

    sha = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip() or "master"
    (out / "index.html").write_text(PAGE.format(
        cards="\n".join(cards), repo=REPO, sha=sha, short=sha[:7],
        date=time.strftime("%Y-%m-%d", time.gmtime()), count=len(docs),
    ), encoding="utf-8")
    (out / ".nojekyll").write_text("")
    if failures:
        print("failed: " + ", ".join(failures), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

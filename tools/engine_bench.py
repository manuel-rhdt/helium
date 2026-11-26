#!/usr/bin/env python3
"""Compares Umbruch with headless Chrome and WeasyPrint: wall time and memory, on documents whose
page breaking is difficult.

    python3 tools/engine_bench.py [--reps 3] [--engines umbruch,chrome,weasyprint]
                                  [--suites synthetic,scaling,gallery] [--only NAME,...]
                                  [--work DIR] [--json results.json]

Needs `pdfinfo` and `pdftotext` (poppler-utils), `psutil`, `weasyprint` (a command on PATH, or
`--weasyprint CMD`) and Chrome (`--chrome PATH`, default `google-chrome`). Umbruch is
`target/release/umbruch` (`--umbruch`).

Every run is a fresh process, started the way a user would start it: `umbruch in.html out.pdf`,
`weasyprint in.html out.pdf`, `chrome --headless --print-to-pdf`. So the times include the start
of the program (Chrome: the browser; WeasyPrint: the Python interpreter and its imports); the
`empty` document measures exactly that.

Memory is the peak of the sum of the proportional set size (PSS) of the process and of all of its
descendants, sampled every few milliseconds. PSS counts a page that processes share once, which is
what makes Chrome (several processes that map the same libraries) comparable to a single
process. A run shorter than the sampling interval can undershoot the peak a little.

Besides the time, the report has the number of pages, the number of words in the PDF (a check that
nothing was dropped: the engines must agree up to the hyphenation and the like) and the mean fill
of the pages that are not the last (how far down the text of a page reaches, as a part of the
page area: a rough measure of how well the page breaks use the pages).

The documents of the suite `synthetic` are generated from a seed. They set the same fonts and
sizes for all engines and turn hyphenation off (headless Chrome has no hyphenation dictionaries,
so with `hyphens: auto` the line breaks, and then the page breaks, would differ for that reason).
"""
import argparse, json, random, re, shutil, statistics, subprocess, tempfile, time
from pathlib import Path

import psutil

REPO = Path(__file__).resolve().parent.parent
WORDS = ("the of and to in a is that it was for on are as with his they at be this from have or by one had not but what all "
         "were when we there can an your which their said if do will each about how up out them then she many some so these "
         "would other into has more her two like him see time could no make than first been its who now people my made over "
         "did down only way find use may water long little very after words called just where most know extraordinary "
         "understanding unfortunately circumstances notwithstanding acquaintance disappointment neighbourhood").split()

# ------------------------------------------------------------------------------------ documents

def sentence(rng, lo=6, hi=22):
    words = [rng.choice(WORDS) for _ in range(rng.randint(lo, hi))]
    return words[0].capitalize() + " " + " ".join(words[1:]) + "."

def text(rng, sentences):
    return " ".join(sentence(rng) for _ in range(sentences))

def paragraph(rng, lo=2, hi=7):
    return "<p>" + text(rng, rng.randint(lo, hi)) + "</p>\n"

def title(rng, lo=1, hi=5):
    return " ".join(rng.choice(WORDS) for _ in range(rng.randint(lo, hi))).capitalize()

def page(extra="", size="A4", margin="20mm"):
    # A4, 11pt on 15pt: about 50 lines to a page. DejaVu Serif is built into Umbruch and installed here
    return f"""<!DOCTYPE html><html lang="en"><meta charset="utf-8"><title>bench</title><style>
@page {{ size: {size}; margin: {margin} }}
html {{ font: 11pt/15pt "DejaVu Serif"; hyphens: manual }}
body {{ margin: 0; orphans: 2; widows: 2 }}
p {{ margin: 0 0 6pt; text-align: justify }}
h1 {{ font: bold 20pt/26pt "DejaVu Sans"; margin: 0 0 12pt; break-after: avoid }}
h2 {{ font: bold 14pt/20pt "DejaVu Sans"; margin: 14pt 0 6pt; break-after: avoid }}
h3 {{ font: bold 11pt/15pt "DejaVu Sans"; margin: 10pt 0 4pt; break-after: avoid }}
{extra}</style>\n"""

def prose(rng, n):
    # plain flowing text: the baseline for every engine. Headings that keep with the next
    # paragraph, orphans and widows of 3
    out = [page("body { orphans: 3; widows: 3 }")]
    for i in range(n):
        if i % 10 == 0:
            out.append(f"<h2>{title(rng)}</h2>\n")
        out.append(paragraph(rng, 2, 9))
    return "".join(out)

def chapters(rng, n):
    # a forced break before every chapter (to the right: blank left pages), mirrored margins and
    # a chain of two headings that must stay with their text
    out = [page("@page :left { margin-right: 30mm } @page :right { margin-left: 30mm }"
                " h1 { break-before: right; margin-top: 60mm }")]
    for c in range(max(1, n // 25)):
        out.append(f"<h1>Chapter {c + 1}: {title(rng)}</h1>\n")
        for s in range(rng.randint(2, 5)):
            out.append(f"<h2>{title(rng)}</h2><h3>{title(rng)}</h3>\n")
            out += [paragraph(rng) for _ in range(rng.randint(2, 8))]
    return "".join(out)

def table(rng, n):
    # one table of 3n rows over many pages: the header is repeated, rows are not split, cells of
    # one to several lines (the row heights vary, so the rows never fill a page exactly)
    out = [page("table { border-collapse: collapse; width: 100%; table-layout: fixed } th, td { border: 0.5pt solid #888; padding: 2pt 4pt; "
                "text-align: left; vertical-align: top; font-size: 9pt; line-height: 12pt } thead th { background: #dde } "
                "tr { break-inside: avoid } caption { font-weight: bold; margin-bottom: 6pt }")]
    out.append("<table><caption>Measurements</caption><colgroup><col style='width:8%'><col style='width:22%'><col style='width:55%'><col style='width:15%'></colgroup><thead><tr><th>No.<th>Name<th>Description<th>Value</tr></thead><tbody>\n")
    for i in range(n * 3):
        out.append(f"<tr><td>{i + 1}<td>{title(rng, 1, 3)}<td>{text(rng, rng.choice([1, 1, 2, 4]))}<td>{rng.random() * 1000:.3f}</tr>\n")
    out.append("</tbody></table>")
    return "".join(out)

def tables_in_text(rng, n):
    # many short tables (3 to 40 rows) between paragraphs: the table either fits at the foot of a
    # page, splits or goes on the next one
    out = [page("table { border-collapse: collapse; width: 100%; table-layout: fixed; margin: 6pt 0 10pt } td, th { border: 0.5pt solid #888; padding: 2pt 4pt; "
                "font-size: 9pt; line-height: 12pt } tr { break-inside: avoid } thead th { background: #dde } caption { caption-side: top; "
                "font-weight: bold; text-align: left; break-after: avoid }")]
    for i in range(n // 3):
        out.append(paragraph(rng, 2, 6))
        out.append(f"<table><caption>Table {i + 1}</caption><colgroup><col style='width:25%'><col style='width:60%'><col style='width:15%'></colgroup><thead><tr><th>Key<th>Text<th>Value</tr></thead><tbody>")
        for _ in range(rng.randint(3, 40)):
            out.append(f"<tr><td>{title(rng, 1, 2)}<td>{text(rng, rng.choice([1, 1, 2]))}<td>{rng.randint(0, 99999)}</tr>")
        out.append("</tbody></table>\n")
    return "".join(out)

def keep(rng, n):
    # boxes that may not be split (figures with a caption, of 30pt up to 480pt, a page has 750pt),
    # with headings that must stay with them, between paragraphs: whether a box goes on this page
    # or the next decides what happens to the pages after it
    out = [page("figure { margin: 8pt 0; break-inside: avoid } figure .art { height: var(--h); background: #cce; border: 0.5pt solid #446 } "
                "figcaption { font: italic 9pt/12pt 'DejaVu Sans'; padding-top: 3pt } .note { break-inside: avoid; border: 1pt solid #a44; "
                "padding: 4pt 8pt; margin: 8pt 0 }")]
    for i in range(n // 2):
        out.append(f"<h2>{title(rng)}</h2>\n")
        out += [paragraph(rng, 1, 5) for _ in range(rng.randint(0, 3))]
        if rng.random() < 0.6:
            out.append(f'<figure><div class="art" style="--h:{rng.randint(30, 480)}pt"></div><figcaption>Figure {i + 1}. {sentence(rng)}</figcaption></figure>\n')
        else:
            out.append(f'<div class="note"><p>{text(rng, rng.randint(2, 14))}</p></div>\n')
        out += [paragraph(rng, 1, 4) for _ in range(rng.randint(0, 2))]
    return "".join(out)

def lists(rng, n):
    # nested lists and quotes, list items that may not be split, and code blocks that may
    out = [page("li { break-inside: avoid; margin: 0 0 3pt } ul, ol { margin: 4pt 0; padding-left: 18pt } blockquote { margin: 6pt 0 6pt 16pt; "
                "padding-left: 8pt; border-left: 2pt solid #999; break-inside: avoid } pre { font: 8pt/10pt 'DejaVu Sans Mono'; background: #eee; "
                "padding: 4pt; margin: 6pt 0; white-space: pre-wrap }")]
    def items(depth):
        o, c = ("<ul>", "</ul>") if depth % 2 == 0 else ("<ol>", "</ol>")
        s = o
        for _ in range(rng.randint(2, 5)):
            s += "<li>" + text(rng, rng.randint(1, 3))
            if depth < 3 and rng.random() < 0.4:
                s += items(depth + 1)
            s += "</li>"
        return s + c
    for i in range(n // 2):
        out.append(paragraph(rng, 1, 3))
        r = rng.random()
        if r < 0.5:
            out.append(items(0) + "\n")
        elif r < 0.75:
            out.append(f"<blockquote>{paragraph(rng, 2, 6)}</blockquote>\n")
        else:
            out.append("<pre>" + "\n".join(f"line {j:04d}: " + " ".join(rng.choice(WORDS) for _ in range(rng.randint(3, 9)))
                                            for j in range(rng.randint(5, 60))) + "</pre>\n")
    return "".join(out)

def columns(rng, n):
    # two columns of text that go over the pages (the container is taller than a page), with
    # headings that may not end a column, and a three-column block between
    out = [page(".cols { column-count: 2; column-gap: 8mm; margin-bottom: 10pt } .cols3 { column-count: 3; column-gap: 6mm; margin-bottom: 10pt; "
                "font-size: 9pt; line-height: 12pt }")]
    for i in range(max(1, n // 12)):
        out.append(f"<h2>{title(rng)}</h2>\n<div class=cols>")
        for _ in range(rng.randint(5, 14)):
            out.append(f"<h3>{title(rng)}</h3>" + paragraph(rng, 2, 8))
        out.append("</div>\n")
        if i % 3 == 2:
            out.append("<div class=cols3>" + "".join(paragraph(rng, 2, 6) for _ in range(rng.randint(3, 8))) + "</div>\n")
    return "".join(out)

def flexgrid(rng, n):
    # a grid of cards and a flex container, each small enough for one page (Umbruch cannot split a grid or a flex container: the
    # whole container moves to the next page if it does not fit, and what does not fit on a page of its own is cut off)
    out = [page(".grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 8pt; margin-bottom: 12pt } .card { break-inside: avoid; "
                "border: 0.5pt solid #668; padding: 4pt 6pt; font-size: 9pt; line-height: 12pt; background: #f4f4fb } .card h3 { margin: 0 0 3pt } .card p "
                "{ margin: 0 } .flex { display: flex; flex-wrap: wrap; gap: 6pt; margin-bottom: 12pt } .flex > div { flex: 1 1 40mm; border: 0.5pt solid #866; "
                "padding: 3pt; font-size: 9pt; line-height: 12pt }")]
    for i in range(max(1, n // 10)):
        out.append(f"<h2>{title(rng)}</h2>\n<div class=grid>")
        for _ in range(rng.randint(6, 12)):
            out.append(f"<div class=card><h3>{title(rng, 1, 3)}</h3><p>{text(rng, rng.randint(1, 5))}</p></div>")
        out.append("</div>\n<div class=flex>")
        for _ in range(rng.randint(4, 9)):
            out.append(f"<div>{text(rng, rng.randint(1, 4))}</div>")
        out.append("</div>\n")
    return "".join(out)

def mixed(rng, n):
    # everything at once, as in a report: the real-world shape of the problem
    out = [page("figure { margin: 8pt 0; break-inside: avoid } figure .art { height: var(--h); background: #cce } figcaption { font: italic 9pt/12pt 'DejaVu Sans' } "
                "table { border-collapse: collapse; width: 100%; table-layout: fixed; margin: 6pt 0 } td, th { border: 0.5pt solid #888; padding: 2pt 4pt; font-size: 9pt; line-height: 12pt } "
                "tr { break-inside: avoid } thead th { background: #dde } ul { margin: 4pt 0 } li { break-inside: avoid } "
                ".cols { column-count: 2; column-gap: 8mm; margin-bottom: 8pt } h1 { break-before: page }")]
    for c in range(max(1, n // 30)):
        out.append(f"<h1>{title(rng)}</h1>\n")
        for _ in range(rng.randint(3, 6)):
            out.append(f"<h2>{title(rng)}</h2>\n")
            for _ in range(rng.randint(2, 6)):
                r = rng.random()
                if r < 0.4:
                    out.append(paragraph(rng))
                elif r < 0.55:
                    out.append(f'<figure><div class="art" style="--h:{rng.randint(60, 360)}pt"></div><figcaption>{sentence(rng)}</figcaption></figure>\n')
                elif r < 0.7:
                    out.append("<table><colgroup><col style='width:25%'><col style='width:60%'><col style='width:15%'></colgroup><thead><tr><th>A<th>B<th>C</tr></thead><tbody>" + "".join(
                        f"<tr><td>{title(rng, 1, 2)}<td>{text(rng, rng.choice([1, 2]))}<td>{rng.randint(0, 999)}</tr>" for _ in range(rng.randint(3, 30))) + "</tbody></table>\n")
                elif r < 0.85:
                    out.append("<ul>" + "".join(f"<li>{text(rng, rng.randint(1, 3))}</li>" for _ in range(rng.randint(2, 8))) + "</ul>\n")
                else:
                    out.append("<div class=cols>" + "".join(paragraph(rng) for _ in range(rng.randint(3, 9))) + "</div>\n")
    return "".join(out)

# name -> (generator, n for the standard size)
SYNTHETIC = {"prose": (prose, 800), "chapters": (chapters, 600), "table": (table, 600), "tables_in_text": (tables_in_text, 900),
             "keep": (keep, 500), "lists": (lists, 700), "columns": (columns, 700), "flexgrid": (flexgrid, 400), "mixed": (mixed, 600)}
SCALING = {"prose": 100, "table": 75, "keep": 60, "mixed": 75}   # n at scale 1; the series is 1, 2, 4, 8, 16 times that

# ---------------------------------------------------------------------------------------- running

class Engines:
    def __init__(self, args):
        self.args = args
        self.profile = tempfile.mkdtemp(prefix="chrome_profile_")

    def command(self, engine, source: Path, out: Path):
        if engine == "umbruch":
            return [self.args.umbruch, str(source), str(out)]
        if engine == "weasyprint":
            return self.args.weasyprint.split() + [str(source), str(out)]
        if engine == "chrome":
            return [self.args.chrome, "--headless=new", "--no-sandbox", "--disable-gpu", "--no-pdf-header-footer", "--disable-extensions",
                    "--disable-background-networking", "--no-first-run", f"--user-data-dir={self.profile}", f"--print-to-pdf={out}", source.resolve().as_uri()]
        raise ValueError(engine)

def pss(process):
    try:
        with open(f"/proc/{process.pid}/smaps_rollup") as f:
            for line in f:
                if line.startswith("Pss:"):
                    return int(line.split()[1]) * 1024
    except (OSError, psutil.Error):
        pass
    return 0

def run(command, cwd, timeout):
    """(wall seconds, peak PSS of the tree in bytes, exit code, stderr tail)"""
    start = time.perf_counter()
    errfile = tempfile.TemporaryFile()
    child = subprocess.Popen(command, cwd=cwd, stdout=subprocess.DEVNULL, stderr=errfile)
    top = psutil.Process(child.pid)
    peak, code, tree = 0, None, []
    while code is None:
        try:
            tree = [top] + top.children(recursive=True)
        except psutil.Error:
            tree = []
        peak = max(peak, sum(pss(p) for p in tree))
        try:
            code = child.wait(timeout=0.004)
        except subprocess.TimeoutExpired:
            if time.perf_counter() - start > timeout:
                for p in tree:
                    try: p.kill()
                    except psutil.Error: pass
                child.wait()
                return timeout, peak, "timeout", ""
    wall = time.perf_counter() - start
    errfile.seek(0)
    return wall, peak, code, errfile.read().decode(errors="replace")[-300:]

def inspect(pdf: Path):
    """pages, words, mean fill of the pages that are not the last"""
    info = subprocess.run(["pdfinfo", str(pdf)], capture_output=True, text=True).stdout
    m = re.search(r"^Pages:\s+(\d+)", info, re.M)
    size = re.search(r"Page size:\s+([\d.]+) x ([\d.]+)", info)
    boxes = subprocess.run(["pdftotext", "-bbox", str(pdf), "-"], capture_output=True, text=True).stdout
    words = len(re.findall(r"<word ", boxes))
    fills = []
    for chunk in boxes.split("<page ")[1:]:
        ys = [float(y) for y in re.findall(r'yMax="([\d.]+)"', chunk)]
        fills.append(max(ys) if ys else 0.0)
    height = float(size.group(2)) if size else 842.0
    margin = 20 / 25.4 * 72   # 20 mm margins of the synthetic documents
    fill = [(f - margin) / (height - 2 * margin) for f in fills[:-1] if f > 0]
    return (int(m.group(1)) if m else 0), words, (statistics.mean(fill) if fill else None)

def bench(engines, engine_names, name, html: Path, reps, timeout, results, work):
    for engine in engine_names:
        out = work / f"{html.parent.name if html.name == 'index.html' else ''}{html.stem}.{engine}.pdf"
        walls, peaks, status = [], [], "ok"
        for _ in range(reps):
            if out.exists():
                out.unlink()
            wall, peak, code, err = run(engines.command(engine, html, out), html.parent, timeout)
            if code != 0 or not out.exists():
                status = "timeout" if code == "timeout" else f"fail({code}): {err.strip()[-120:]}"
                break
            walls.append(wall); peaks.append(peak)
        row = {"doc": name, "engine": engine, "status": status}
        if walls:
            pages, words, fill = inspect(out)
            row.update(wall=statistics.median(walls), wall_min=min(walls), wall_max=max(walls), mem=max(peaks), pages=pages, words=words, fill=fill)
        results.append(row)
        print(fmt_row(row), flush=True)

def fmt_row(r):
    if "wall" not in r:
        return f"{r['doc']:<24}{r['engine']:<12}{r['status']}"
    fill = f"{r['fill'] * 100:5.1f}%" if r["fill"] is not None else "    -"
    return f"{r['doc']:<24}{r['engine']:<12}{r['wall']:8.2f}s {r['mem'] / 2**20:8.0f} MiB {r['pages']:5} pages {r['words']:8} words  fill {fill}"

def table_report(results, engine_names, docs):
    by = {(r["doc"], r["engine"]): r for r in results}
    ratios = len(engine_names) == 3 and engine_names[0] == "umbruch"
    for what, label in (("wall", "wall time [s] (median)"), ("mem", "peak memory [MiB] (PSS of the process tree)"), ("pages", "pages"),
                        ("words", "words in the PDF"), ("fill", "mean fill of the pages but the last [%] (the margins of the synthetic documents)")):
        print(f"\n{label}\n{'':<24}" + "".join(f"{e:>13}" for e in engine_names) + (f"{engine_names[1] + '/um':>13}{engine_names[2] + '/um':>13}" if what in ("wall", "mem") and ratios else ""))
        for doc in docs:
            cells, vals = [], {}
            for e in engine_names:
                r = by.get((doc, e))
                if not r or what not in r or r[what] is None:
                    cells.append(f"{(r or {}).get('status', '-')[:11]:>13}"); continue
                v = r[what]; vals[e] = v
                cells.append({"wall": lambda: f"{v:13.2f}", "mem": lambda: f"{v / 2**20:13.0f}", "pages": lambda: f"{v:13.0f}",
                              "words": lambda: f"{v:13.0f}", "fill": lambda: f"{v * 100:13.1f}"}[what]())
            extra = ""
            if what in ("wall", "mem") and ratios:
                for other in engine_names[1:]:
                    extra += f"{vals[other] / vals['umbruch']:12.1f}x" if "umbruch" in vals and other in vals else f"{'-':>13}"
            print(f"{doc:<24}" + "".join(cells) + extra)

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--reps", type=int, default=3)
    ap.add_argument("--engines", default="umbruch,chrome,weasyprint")
    ap.add_argument("--suites", default="synthetic,scaling,gallery")
    ap.add_argument("--only", default="", help="comma separated document names")
    ap.add_argument("--seed", default="bench")
    ap.add_argument("--scale", type=float, default=1.0, help="multiplies the size of the synthetic documents")
    ap.add_argument("--work", default=None)
    ap.add_argument("--json", default=None)
    ap.add_argument("--timeout", type=float, default=600)
    ap.add_argument("--umbruch", default=str(REPO / "target/release/umbruch"))
    ap.add_argument("--weasyprint", default="weasyprint")
    ap.add_argument("--chrome", default="google-chrome")
    args = ap.parse_args()
    names = args.engines.split(",")
    work = Path(args.work or tempfile.mkdtemp(prefix="engine_bench_")); work.mkdir(parents=True, exist_ok=True)
    engines = Engines(args)
    only = set(filter(None, args.only.split(",")))
    results, docs = [], []
    def go(name, html: Path):
        if only and name not in only:
            return
        docs.append(name)
        bench(engines, names, name, html, args.reps, args.timeout, results, work)
    suites = args.suites.split(",")
    (work / "empty.html").write_text(page() + "<p>Hello.</p>")
    go("empty", work / "empty.html")
    if "synthetic" in suites:
        for name, (make, n) in SYNTHETIC.items():
            path = work / f"{name}.html"
            path.write_text(make(random.Random(f"{args.seed}{name}"), int(n * args.scale)))
            go(name, path)
    if "scaling" in suites:
        for name, n in SCALING.items():
            for k in (1, 2, 4, 8, 16):
                path = work / f"{name}_x{k}.html"
                path.write_text(SYNTHETIC[name][0](random.Random(f"{args.seed}{name}"), n * k))
                go(f"{name}_x{k}", path)
    if "gallery" in suites:
        order = (REPO / "gallery/order.txt").read_text().split()
        for name in order:
            index = REPO / "gallery" / name / "index.html"
            if index.exists():
                go(f"gallery/{name}", index)
    print("\n" + "=" * 100)
    table_report(results, names, docs)
    if args.json:
        Path(args.json).write_text(json.dumps(results, indent=1))
    shutil.rmtree(engines.profile, ignore_errors=True)

if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""How many lines do the pages (and the columns) of long generated documents have?

    python3 tools/page_uniformity.py [--docs N] [--kinds book,columns] [--pages P]

Every document is set on a grid of 20px lines, with pages and columns of 24 lines, and rendered
with several settings (`looseness`, the tolerances, orphans and widows). The report gives, for the pages
that do not end the document or a chapter (a forced break), how many have all 24 lines, how many lines the
others lack, and the badness of the lines that the document pays for it (see `looseness_bench.py`).
The columns of the kind `columns` are the columns of a container with two columns that goes over the pages.
"""
import argparse, collections, json, os, random, re, subprocess, sys, tempfile
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import looseness_bench as bench

LINE, FULL = 20.0, 24

def head(extra="", rules=2):
    return f"""<!DOCTYPE html><html lang=en><meta charset=utf-8><title>b</title><style>
@page {{ size: 400px 560px; margin: 40px }}
body {{ margin: 0; font: 14px/{LINE}px serif; orphans: {rules}; widows: {rules} }}
p {{ margin: 0; text-align: justify; hyphens: auto; text-indent: 1.3em }}
h2 {{ margin: 0 0 {LINE}px; font: bold 14px/{LINE}px serif; break-before: page; break-after: avoid }}
h3 {{ margin: {LINE}px 0 0; font: bold 14px/{LINE}px serif; break-after: avoid }}
blockquote {{ margin: {LINE}px 0; padding: 0 20px }}
blockquote p {{ text-indent: 0 }}
{extra}</style>"""

def paragraph(rng, low=2, high=7):
    return bench.paragraph(rng)

def book(rng, rules=2):
    out = [head(rules=rules)]
    for chapter in range(rng.randint(5, 9)):
        out.append(f"<h2>Chapter {chapter + 1}</h2>")
        for _ in range(rng.randint(25, 60)):
            roll = rng.random()
            if roll < 0.06:
                out.append("<h3>" + " ".join(rng.choice(bench.WORDS) for _ in range(rng.randint(1, 4))).capitalize() + "</h3>")
            if roll > 0.96:
                out.append("<blockquote>" + paragraph(rng) + "</blockquote>")
            else:
                out.append(paragraph(rng))
    return "".join(out)

def two_columns(rng, rules=2):
    out = [head("@page { size: 800px 560px; margin: 40px } .cols { column-count: 2; column-gap: 24px }", rules)]
    out.append("<div class=cols>")
    for _ in range(rng.randint(200, 320)):
        out.append(paragraph(rng))
    out.append("</div>")
    return "".join(out)

KINDS = {"book": book, "columns": two_columns}

# (name, looseness, knobs, rules)
SETTINGS = [
    ("looseness 0", 0, {}, 2),
    ("looseness 1", 1, {}, 2),
    ("looseness 2, variant tol 1000", 2, {"VARIANT_TOLERANCE": "1000"}, 2),
    ("looseness 2, both open", 2, {"VARIANT_TOLERANCE": "100000", "LINE_TOLERANCE": "20000"}, 2),
    ("looseness 2, cap 1000, rate 2000", 2, {"VARIANT_TOLERANCE": "1000", "LINE_TOLERANCE": "2000"}, 2),
    ("looseness 2, cap 1000, rate 20000", 2, {"VARIANT_TOLERANCE": "1000", "LINE_TOLERANCE": "20000"}, 2),
    ("looseness 4, cap 1000, rate 20000", 4, {"VARIANT_TOLERANCE": "1000", "LINE_TOLERANCE": "20000"}, 2),
    ("looseness 0, orphans/widows 1", 0, {}, 1),
    ("looseness 0, orphans/widows 3", 0, {}, 3),
    ("looseness 0, no earlier fragments", 0, {"EARLIER_FRAGMENTS": "0"}, 2),
    ("looseness 1, no earlier fragments", 1, {"EARLIER_FRAGMENTS": "0"}, 2),
    ("looseness 2 both open, no earlier", 2, {"VARIANT_TOLERANCE": "100000", "LINE_TOLERANCE": "20000", "EARLIER_FRAGMENTS": "0"}, 2),
]

def run(html, looseness, knobs, directory, name):
    html = html.replace("</style>", f"body {{ looseness: {looseness} }}</style>", 1)
    path = os.path.join(directory, name + ".html")
    open(path, "w").write(html)
    env = {**os.environ, "RUST_LOG": "info", **{"HELIUM_" + k: v for k, v in knobs.items()}}
    process = subprocess.run([bench.BIN, "--no-system-fonts", "--dump-pagination", path[:-5] + ".json", path, path[:-5] + ".pdf"],
                             capture_output=True, text=True, env=env)
    if process.returncode:
        sys.exit(process.stderr)
    report = json.load(open(path[:-5] + ".json"))
    match = re.search(r"LINES: (\d+) lines, mean badness ([\d.]+), worst ([\d.]+), (\d+) over 100", process.stderr)
    return report, (float(match.group(2)), int(match.group(4)), int(match.group(1))) if match else (0, 0, 0)

def measure(kind, report):
    """The line counts of the pages (or of the columns) that should be full."""
    counts = []
    pages = report["pages"]
    for number, page in enumerate(pages):
        if number + 1 == len(pages) or page["ends_with"] == "Forced":
            continue
        if kind == "columns":
            for container in page["containers"]:
                # (the columns of a fragment that goes on, and not the balanced last fragment)
                if container["ends_in_container"]:
                    counts += [round(used / LINE) for used in container["columns"]]
        else:
            counts.append(round(page["used"] / LINE))
    return counts

def summarize(name, counts, quality):
    n = len(counts)
    deficit = sorted(FULL - c for c in counts)
    full = sum(1 for d in deficit if d == 0)
    mean = sum(deficit) / n
    p95 = deficit[int(0.95 * (n - 1))]
    histogram = collections.Counter(deficit)
    shape = " ".join(f"{d}:{histogram[d]}" for d in sorted(histogram)[:5])
    print(f"  {name:<32} {n:>6} {100 * full / n:7.1f}% {mean:6.2f} {p95:4} {max(deficit):4}   badness {quality[0]:.1f}, over 100: {quality[1]:.0f}   [{shape}]")

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--docs", type=int, default=15)
    parser.add_argument("--kinds", default="book,columns")
    parser.add_argument("--only", default="", help="a part of the name of the settings to run")
    args = parser.parse_args()
    directory = tempfile.mkdtemp(prefix="uniform_")
    for kind in args.kinds.split(","):
        print(f"\n{kind}: {args.docs} documents; pages (columns) that should have {FULL} lines")
        print(f"  {'setting':<32} {'count':>6} {'full':>8} {'missing':>6} {'p95':>4} {'max':>4}")
        for name, looseness, knobs, rules in SETTINGS:
            if args.only and args.only not in name:
                continue
            counts, quality = [], [0.0, 0.0]
            for seed in range(args.docs):
                html = KINDS[kind](random.Random(f"{kind}{seed}"), rules)
                report, stats = run(html, looseness, knobs, directory, f"{kind}{seed}")
                counts += measure(kind, report)
                quality[0] += stats[0] / args.docs
                quality[1] += stats[1] / args.docs
            summarize(name, counts, quality)

if __name__ == "__main__":
    main()

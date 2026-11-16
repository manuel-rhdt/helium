#!/usr/bin/env python3
"""Measures what `looseness` does to the pages of generated documents.

    python3 tools/looseness_bench.py [--seeds N] [--levels 0,1,2] [--keep DIR] [FILE.html ...]

For each kind of document (and each seed) the document is rendered with `looseness` 0, 1, 2, ...
set on the body, and the report gives, per kind and level, the mean of:

  pages      the number of pages
  room       the room left at the foot of the pages that are not the last, in lines
  loose      the lines that are broken worse than a badness of 100 (TeX's measure)
  badness    the mean badness of a line
  gain       in how many of the documents there are fewer pages than at looseness 0 / more

The documents are generated from a seed, so a run is repeatable. FILEs are rendered the same way
(a `<style>` for `body { looseness: N }` is added at the end of the head).

The kinds are the cases that decide whether looseness is of use: `flow` is plain prose, where a
page can be filled by cutting a paragraph almost anywhere; `chapters` has a forced break before
every chapter, whose last page is free; `figures` has boxes that cannot be broken.
"""
import argparse, json, os, random, re, subprocess, sys, tempfile

BIN = os.environ.get("HELIUM", os.path.join(os.path.dirname(__file__), "..", "target", "release", "helium"))
LINE, TOP, HEIGHT = 20.0, 40.0, 480.0
WORDS = ("the of and to in a is that it was for on are as with his they at be this from have or by one had not but what all "
         "were when we there can an your which their said if do will each about how up out them then she many some so these "
         "would other into has more her two like him see time could no make than first been its who now people my made over "
         "did down only way find use may water long little very after words called just where most know extraordinary "
         "understanding unfortunately circumstances notwithstanding acquaintance disappointment neighbourhood").split()

def sentence(rng):
    words = [rng.choice(WORDS) for _ in range(rng.randint(6, 22))]
    return words[0].capitalize() + " " + " ".join(words[1:]) + "."

def paragraph(rng):
    return "<p>" + " ".join(sentence(rng) for _ in range(rng.randint(2, 7))) + "</p>"

def head(extra=""):
    return f"""<!DOCTYPE html><html lang=en><meta charset=utf-8><title>b</title><style>
@page {{ size: 400px 560px; margin: {TOP}px }}
body {{ margin: 0; font: 14px/{LINE}px serif; orphans: 2; widows: 2 }}
p {{ margin: 0; text-align: justify; hyphens: auto; text-indent: 1.3em }}
h2 {{ margin: 0; font: bold 14px/{LINE}px serif; break-before: page }}
.fig {{ height: var(--h); margin: 10px 0; background: #ccd; break-inside: avoid }}
{extra}</style>"""

def flow(rng):
    return head() + "".join(paragraph(rng) for _ in range(rng.randint(50, 90)))

def chapters(rng):
    out = [head()]
    for c in range(rng.randint(8, 14)):
        out.append(f"<h2>Chapter {c + 1}</h2>")
        out += [paragraph(rng) for _ in range(rng.randint(4, 30))]
    return "".join(out)

def figures(rng):
    out = [head()]
    for _ in range(rng.randint(15, 30)):
        out += [paragraph(rng) for _ in range(rng.randint(1, 4))]
        out.append(f'<div class=fig style="--h:{rng.randint(60, 240)}px"></div>')
    return "".join(out)

KINDS = {"flow": flow, "chapters": chapters, "figures": figures}

def render(html, level, directory, name):
    html = re.sub(r"</style>", f"body {{ looseness: {level} }}</style>", html, count=1)
    path = os.path.join(directory, f"{name}_l{level}.html")
    with open(path, "w") as file:
        file.write(html)
    process = subprocess.run([BIN, "--no-system-fonts", "--dump-layout", "-", path, path[:-5] + ".pdf"],
                             capture_output=True, text=True, env={**os.environ, "RUST_LOG": "info"})
    if process.returncode:
        sys.exit(process.stderr)
    dump = json.loads(process.stdout)
    # the lowest edge of what is on a page
    bottom = {}
    for text in dump["paint"]["text"]:
        newlines = sum(1 for op in text["ops"] if op["op"] == "newline")
        edge = text["y"] + newlines * LINE + LINE
        bottom[text["page"]] = max(bottom.get(text["page"], 0.0), edge)
    for rect in dump["paint"]["rects"]:
        if rect["color"][:3] == [204, 204, 221]:
            bottom[rect["page"]] = max(bottom.get(rect["page"], 0.0), rect["y"] + rect["height"] + 10)
    pages = max(bottom) + 1 if bottom else 1
    room = sum(max(0.0, TOP + HEIGHT - bottom.get(page, TOP)) for page in range(pages - 1)) / LINE
    match = re.search(r"LINES: (\d+) lines, mean badness ([\d.]+), worst ([\d.]+), (\d+) over 100", process.stderr)
    lines, badness, _, loose = (float(x) for x in match.groups()) if match else (0, 0, 0, 0)
    return {"pages": pages, "room": room, "loose": loose, "badness": badness, "lines": lines}

def report(title, results, levels):
    print(f"\n{title} ({len(results)} documents)")
    print(f"{'looseness':>9} {'pages':>8} {'room':>8} {'lines':>8} {'loose':>8} {'badness':>8} {'fewer pages':>12} {'more pages':>11}")
    for level in levels:
        rows = [result[level] for result in results]
        mean = lambda key: sum(row[key] for row in rows) / len(rows)
        fewer = sum(1 for result in results if result[level]["pages"] < result[levels[0]]["pages"])
        more = sum(1 for result in results if result[level]["pages"] > result[levels[0]]["pages"])
        print(f"{level:>9} {mean('pages'):8.2f} {mean('room'):8.2f} {mean('lines'):8.1f} {mean('loose'):8.1f} {mean('badness'):8.2f} {fewer:12} {more:11}")

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seeds", type=int, default=10)
    parser.add_argument("--levels", default="0,1,2")
    parser.add_argument("--keep")
    parser.add_argument("files", nargs="*")
    args = parser.parse_args()
    levels = [int(level) for level in args.levels.split(",")]
    directory = args.keep or tempfile.mkdtemp(prefix="looseness_")
    os.makedirs(directory, exist_ok=True)
    if args.files:
        results = []
        for file in args.files:
            html = open(file).read()
            results.append({level: render(html, level, directory, os.path.basename(file)[:-5]) for level in levels})
        report("files", results, levels)
        return
    for kind, make in KINDS.items():
        results = []
        for seed in range(args.seeds):
            html = make(random.Random(f"{kind}{seed}"))
            results.append({level: render(html, level, directory, f"{kind}{seed}") for level in levels})
        report(kind, results, levels)

main()

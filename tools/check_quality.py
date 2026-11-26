#!/usr/bin/env python3
"""Checks the output of umbruch for generated documents with and without a setting, for the
correctness and the typographic quality of the pages (not the speed):

    python3 tools/check_quality.py [--seeds N] [--kinds a,b] [--knob NAME=VALUE ...] [--against NAME=VALUE ...]

The documents are the ones of `looseness_bench.py`. Every document is rendered with the knobs of
`--knob` (the first setting) and with those of `--against` (the second), and these are checked for
both, and compared:

  words      the words of the pages are the same words (none lost, none twice)
  inside     every word is inside the content area of its page
  warnings   the layout agrees with the paginator (the warning "page N is ... high" is not logged)
  rules      no page breaks orphans or widows, or ends where a break should be avoided
  ragged     (for information) the fragments of containers whose columns are more than a line and
             a half apart in the extent of their content: a paragraph of three lines cannot be
             split between two columns by rules of two, so it can be the best there is
"""
import argparse, collections, json, os, random, re, subprocess, sys, tempfile
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import looseness_bench as bench

def run(html, name, directory, knobs):
    path = os.path.join(directory, name + ".html")
    open(path, "w").write(html)
    env = {**os.environ, "RUST_LOG": "warn", **{"UMBRUCH_" + k: v for k, v in knobs.items()}}
    process = subprocess.run([bench.BIN, "--no-system-fonts", "--dump-pagination", path[:-5] + ".json", path, path[:-5] + ".pdf"],
                             capture_output=True, text=True, env=env)
    if process.returncode:
        sys.exit(process.stderr)
    report = json.load(open(path[:-5] + ".json"))
    words = subprocess.run(["pdftotext", "-bbox", path[:-5] + ".pdf", "-"], capture_output=True, text=True).stdout
    return process.stderr, report, words

def check(html, stderr, report, bbox):
    problems = collections.Counter()
    sizes = re.search(r"@page\s*\{\s*size:\s*([\d.]+)px\s+([\d.]+)px;\s*margin:\s*([\d.]+)px", html)
    # (the last @page rule is the one that counts)
    for found in re.finditer(r"@page\s*\{\s*size:\s*([\d.]+)px\s+([\d.]+)px", html):
        width, height = float(found.group(1)), float(found.group(2))
    margin = float(re.search(r"margin:\s*([\d.]+)px", html).group(1))
    for page in bbox.split("<page ")[1:]:
        for x0, y0, x1, y1 in re.findall(r'xMin="([\d.]+)" yMin="([\d.]+)" xMax="([\d.]+)" yMax="([\d.]+)"', page):
            x1, y1 = float(x1) / 0.75, float(y1) / 0.75
            if x1 > width - margin + 1 or y1 > height - margin + 6:
                problems["inside"] += 1
    problems["warnings"] += len(re.findall(r"page \d+ is [\d.]+ high", stderr))
    for page in report["pages"]:
        if page["orphan_violation"] or page["widow_violation"] or page["ends_with"] == "Avoid":
            problems["rules"] += 1
        for container in page["containers"]:
            columns = container["columns"]
            if len(columns) < 2:
                continue
            gap = max(columns) - min(columns)
            if container["ends_in_container"]:
                # (the columns of a fragment that goes on are filled in turn: the last may be short
                # if the content ends where the rules need it)
                if gap > 2 * 20 + 0.5 and len(columns) == 2 and False:
                    problems["ragged (info)"] += 1
            elif gap > 1.5 * 20 + 0.5:
                problems["ragged (info)"] += 1
    return problems

def words_of(bbox):
    return collections.Counter(re.findall(r">([^<]+)</word>", bbox))

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seeds", type=int, default=20)
    parser.add_argument("--kinds", default="columns,headings,balanced")
    parser.add_argument("--knob", action="append", default=[])
    parser.add_argument("--against", action="append", default=[])
    args = parser.parse_args()
    knobs = [dict(k.partition("=")[::2] for k in group) for group in (args.knob, args.against)]
    directory = tempfile.mkdtemp(prefix="quality_")
    for kind in args.kinds.split(","):
        totals = [collections.Counter(), collections.Counter()]
        different_words = 0
        documents = 0
        for seed in range(args.seeds):
            html = bench.KINDS[kind](random.Random(f"{kind}{seed}"))
            results = [run(html, f"{kind}{seed}_{n}", directory, knobs[n]) for n in (0, 1)]
            for n, (stderr, report, bbox) in enumerate(results):
                totals[n] += check(html, stderr, report, bbox)
            if words_of(results[0][2]) != words_of(results[1][2]):
                different_words += 1
            documents += 1
        print(f"{kind} ({documents} documents): documents whose words differ between the settings: {different_words}")
        for n, name in enumerate(("first", "against")):
            print(f"  {name:>8} {dict(totals[n]) or 'no problems'}")

if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Collects data about the columns of containers that go over pages, on generated documents of real text.

    python3 tools/columns_study.py [--docs N] [--kinds a,b] [--only NAME] [--csv FILE]

The text is the novel of `gallery/book`. The type is 10pt on 12pt leading, and a page has 30 lines. A column is 24em wide with 2 columns (a journal) and 17em with
3 (a denser layout), and the documents are:

  two        a container with 2 columns of 24em over many pages (the body of a journal article)
  three      the same with 3 columns of 17em
  headings2  2 columns, with sub-headings that must stay with the text that follows them
  sections3  3 columns, in sections: a full width heading, then a container, which is balanced where it ends

For every fragment of a container (the part of it that is on a page) the report has the lines that every
column has. A fragment that goes on to the next page should have the lines that the room allows in every
column ("fill"); the last fragment of a container is balanced, and its columns should be as equal as
the rules let them be ("balance"). The tables give, by number of columns and by how the fragment starts
(a continuation starts in the middle of a paragraph that the page before cut):

  full       columns with all the lines that the room allows
  missing    the lines that the others lack, on average
  gap        the difference between the tallest and the shortest column of a last fragment, in lines, on
             average, and the share of them that differ by at most one line
  room       the room that is left at the foot of the pages that are not the last, in lines, on average
  rules      pages that break orphans or widows, or end at a place that is to be avoided
"""
import argparse, collections, csv, json, os, random, re, subprocess, sys, tempfile
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import looseness_bench as bench

EM = 10 * 96 / 72  # 10pt in px
LINE = 16.0  # 12pt leading
GAP = 20.0
SOURCE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "gallery", "book", "index.html")

def paragraphs():
    text = open(SOURCE).read()
    found = [re.sub(r"<[^>]+>", "", p).strip() for p in re.findall(r"<p>(.*?)</p>", text, flags=re.S)]
    return [p for p in found if len(p) > 120]

PARAGRAPHS = paragraphs()

def title(rng):
    return " ".join(rng.choice(PARAGRAPHS).split()[:rng.randint(1, 4)]).strip("“”,.;:!?—").capitalize()

def text_of(rng, count):
    start = rng.randrange(0, len(PARAGRAPHS) - count)
    return "".join(f"<p>{p}</p>" for p in PARAGRAPHS[start:start + count])

def head(columns, em_width, rules=2):
    """10pt text on 12pt leading, in columns that are `em_width` em wide, on a page that is as wide as they need."""
    width = round(columns * em_width * EM + (columns - 1) * GAP + 80)
    return f"""<!DOCTYPE html><html lang=en><meta charset=utf-8><title>s</title><style>
@page {{ size: {width}px {40 * 2 + 30 * LINE:.0f}px; margin: 40px }}
body {{ margin: 0; font: 10pt/12pt "TeX Gyre Pagella", Palatino, serif; orphans: {rules}; widows: {rules} }}
.cols {{ column-count: {columns}; column-gap: {GAP:.0f}px; text-align: justify; hyphens: auto }}
.cols p {{ margin: 0; text-indent: 1.3em }}
h2 {{ margin: 0; font: bold 10pt/12pt "TeX Gyre Pagella", Palatino, serif; break-after: avoid }}
h3 {{ margin: 0; font: bold 10pt/12pt "TeX Gyre Pagella", Palatino, serif; break-after: avoid; text-indent: 0 }}
</style>"""

def two(rng, rules=2):
    return head(2, 24, rules) + '<div class=cols>' + text_of(rng, rng.randint(150, 260)) + "</div>"

def three(rng, rules=2):
    return head(3, 17, rules) + '<div class=cols>' + text_of(rng, rng.randint(200, 340)) + "</div>"

def headings2(rng, rules=2):
    out = [head(2, 24, rules), "<div class=cols>"]
    for _ in range(rng.randint(30, 50)):
        out.append(f"<h3>{title(rng)}</h3>" + text_of(rng, rng.randint(2, 5)))
    return "".join(out) + "</div>"

def sections3(rng, rules=2):
    out = [head(3, 17, rules)]
    for _ in range(rng.randint(8, 14)):
        out.append(f"<h2>{title(rng)}</h2><div class=cols>" + text_of(rng, rng.randint(6, 45)) + "</div>")
    return "".join(out)

KINDS = {"two": two, "three": three, "headings2": headings2, "sections3": sections3}

# (name, looseness, knobs)
SETTINGS = [
    ("looseness 0, earlier fragments", 0, {}),
    ("looseness 0, no earlier fragments", 0, {"EARLIER_FRAGMENTS": "0"}),
    ("looseness 1, earlier fragments", 1, {}),
    ("looseness 1, no earlier fragments", 1, {"EARLIER_FRAGMENTS": "0"}),
]

def run(html, looseness, knobs, directory, name):
    html = html.replace("</style>", f"body {{ looseness: {looseness} }}</style>", 1)
    path = os.path.join(directory, name + ".html")
    open(path, "w").write(html)
    env = {**os.environ, **{"HELIUM_" + k: v for k, v in knobs.items()}}
    process = subprocess.run([bench.BIN, "--dump-pagination", path[:-5] + ".json", path, path[:-5] + ".pdf"],
                             capture_output=True, text=True, env=env)
    if process.returncode:
        sys.exit(process.stderr)
    return json.load(open(path[:-5] + ".json"))

def fragments(report):
    """One row for every fragment of a container: where it is and the lines of its columns."""
    rows = []
    pages = report["pages"]
    for number, page in enumerate(pages):
        last_page = number + 1 == len(pages)
        for container in page["containers"]:
            columns = [round(used / LINE) for used in container["columns"]]
            rows.append({
                "page": page["page"],
                "columns": columns,
                "target": int(container["available"] // LINE),
                "starts": container["starts_in_container"],
                "ends": not container["ends_in_container"],  # the last fragment of its container
                "room": page["room"] / LINE,
                "page_end": page["ends_with"],
                "last_page": last_page,
                "rules": page["orphan_violation"] or page["widow_violation"] or page["ends_with"] == "Avoid",
            })
    return rows

def summarize(rows, pages):
    out = {}
    full_rows = [r for r in rows if not r["ends"]]
    deficits = collections.defaultdict(list)
    for r in full_rows:
        key = ("continuation" if r["starts"] else "first", len(r["columns"]))
        deficits[key] += [r["target"] - c for c in r["columns"]]
    last = [r for r in rows if r["ends"] and len(r["columns"]) > 1]
    gaps = [max(r["columns"]) - min(r["columns"]) for r in last]
    return deficits, gaps

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--docs", type=int, default=10)
    parser.add_argument("--kinds", default=",".join(KINDS))
    parser.add_argument("--only", default="")
    parser.add_argument("--csv", default="/tmp/columns_study.csv")
    args = parser.parse_args()
    directory = tempfile.mkdtemp(prefix="colstudy_")
    table = open(args.csv, "w", newline="")
    writer = csv.writer(table)
    writer.writerow(["kind", "setting", "seed", "page", "columns", "target", "starts_in_container", "last_fragment", "page_room_lines", "page_end", "rules"])
    for kind in args.kinds.split(","):
        print(f"\n{kind} ({args.docs} documents)")
        print(f"  {'setting':<36} {'pages':>5} {'fill: full':>10} {'missing':>8} {'cont. full':>10} {'balance: gap':>12} {'<=1 line':>8} {'room':>6} {'rules':>5}")
        for name, looseness, knobs in SETTINGS:
            if args.only and args.only not in name:
                continue
            all_rows, pages, rules, room = [], 0, 0, []
            for seed in range(args.docs):
                html = KINDS[kind](random.Random(f"{kind}{seed}"))
                report = run(html, looseness, knobs, directory, f"{kind}{seed}")
                rows = fragments(report)
                for r in rows:
                    writer.writerow([kind, name, seed, r["page"], " ".join(map(str, r["columns"])), r["target"], int(r["starts"]),
                                     int(r["ends"]), f"{r['room']:.1f}", r["page_end"], int(r["rules"])])
                all_rows += rows
                pages += len(report["pages"])
                rules += sum(1 for p in report["pages"] if p["orphan_violation"] or p["widow_violation"] or p["ends_with"] == "Avoid")
                room += [p["room"] / LINE for p in report["pages"][:-1] if p["ends_with"] != "Forced"]
            deficits, gaps = summarize(all_rows, pages)
            every = [d for ds in deficits.values() for d in ds]
            cont = [d for key, ds in deficits.items() if key[0] == "continuation" for d in ds]
            full = 100 * sum(1 for d in every if d == 0) / max(1, len(every))
            cont_full = 100 * sum(1 for d in cont if d == 0) / max(1, len(cont))
            missing = sum(every) / max(1, len(every))
            gap = sum(gaps) / max(1, len(gaps))
            near = 100 * sum(1 for g in gaps if g <= 1) / max(1, len(gaps))
            print(f"  {name:<36} {pages:>5} {full:9.1f}% {missing:8.2f} {cont_full:9.1f}% {gap:12.2f} {near:7.1f}% {sum(room) / max(1, len(room)):6.2f} {rules:>5}")
    table.close()
    print(f"\nthe fragments are in {args.csv}")

if __name__ == "__main__":
    main()

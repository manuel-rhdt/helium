#!/usr/bin/env python3
"""Shows the worst-set lines of a document, as images with their badness.

    python3 tools/worst_lines.py DOC.html OUT.png [--count N] [--columns N] [--dpi D] [--looseness L] [--knob NAME=VALUE ...]

The document is rendered with `looseness` L on the body and the knobs (as `looseness_bench.py`), and
the pages that have the lines with the greatest badness (TeX's measure: 100 for a line that is stretched or
shrunk by its whole stretch) are shown whole, side by side, with the badness of their worst line. The last line of a paragraph is not justified and has none.
"""
import argparse, json, math, os, re, subprocess, sys, tempfile
from PIL import Image, ImageDraw, ImageFont
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import looseness_bench as bench

def lines_of(report):
    found = []
    for page in report["pages"]:
        for paragraph in page["paragraphs"]:
            if not paragraph.get("justified", False):
                continue  # (the badness of a line that is not justified says nothing)
            boxes = paragraph.get("boxes", [])
            count = len(paragraph["badness"])
            for index, badness in enumerate(paragraph["badness"]):
                if index + 1 == count and not paragraph["cut_at_end"]:
                    continue  # (the last line of a paragraph: not justified)
                if index >= len(boxes):
                    continue
                found.append((math.inf if badness is None else badness, page["page"], paragraph["entity"], index, boxes[index]))
    return found

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("document")
    parser.add_argument("output")
    parser.add_argument("--count", type=int, default=12)
    parser.add_argument("--looseness", type=int, default=0)
    parser.add_argument("--knob", action="append", default=[])
    parser.add_argument("--dpi", type=int, default=70)
    parser.add_argument("--columns", type=int, default=4, help="pages in a row")
    args = parser.parse_args()
    directory = tempfile.mkdtemp(prefix="worst_")
    html = open(args.document).read()
    html = html.replace("</style>", f"body {{ looseness: {args.looseness} }}</style>", 1)
    path = os.path.join(directory, "doc.html")
    open(path, "w").write(html)
    # (the document may refer to files next to it)
    base = os.path.dirname(os.path.abspath(args.document))
    shutil_copy = subprocess.run(["cp", "-r", "-n", base + "/.", directory], capture_output=True)
    env = {**os.environ, **{"HELIUM_" + k.removeprefix("HELIUM_").partition("=")[0]: k.partition("=")[2] for k in args.knob}}
    subprocess.run([bench.BIN, "--no-system-fonts", "--dump-pagination", os.path.join(directory, "r.json"), path,
                    os.path.join(directory, "doc.pdf")], check=True, capture_output=True, env=env)
    report = json.load(open(os.path.join(directory, "r.json")))
    found = sorted(lines_of(report), key=lambda line: -line[0])
    # (a line with an infinite badness is a short last line of a paragraph, as a signature, which no
    # stretch of its spaces could fill: it is counted and not shown)
    infinite = [line for line in found if line[0] >= 10000]
    found = [line for line in found if line[0] < 10000]
    all_lines = len(found)
    chosen, seen = [], set()  # (one line of a page)
    for line in found:
        # (one line of a paragraph, and of a page, is enough)
        key = line[1]
        if key in seen:
            continue
        seen.add(key)
        chosen.append(line)
        if len(chosen) == args.count:
            break
    # (the whole page of every one of the worst lines, once; no marks)
    font = ImageFont.load_default(size=15)
    pages = []
    for badness, page, entity, index, box in chosen:
        if any(shown == page for shown, _ in pages):
            continue
        subprocess.run(["pdftoppm", "-png", "-r", str(args.dpi), "-f", str(page), "-l", str(page), "-singlefile",
                        os.path.join(directory, "doc.pdf"), os.path.join(directory, f"p{page}")], check=True)
        pages.append((page, (badness, Image.open(os.path.join(directory, f"p{page}.png")).convert("RGB"))))
    per_row = args.columns
    cell_w, cell_h = pages[0][1][1].size
    rows = (len(pages) + per_row - 1) // per_row
    canvas = Image.new("RGB", (per_row * (cell_w + 16) + 16, rows * (cell_h + 34) + 40), "white")
    draw = ImageDraw.Draw(canvas)
    knobs = ", ".join(args.knob) or "default tolerances"
    draw.text((16, 10), f"Pages with the worst lines of {all_lines} justified lines: looseness {args.looseness}, {knobs}", fill=(0, 0, 0), font=font)
    for number, (page, (badness, image)) in enumerate(pages):
        x, y = 16 + (number % per_row) * (cell_w + 16), 40 + (number // per_row) * (cell_h + 34)
        label = "a line that cannot be set" if math.isinf(badness) else f"worst line: badness {badness:.0f}"
        draw.text((x, y), f"page {page}: {label}", fill=(0, 0, 0), font=font)
        canvas.paste(image, (x, y + 22))
    canvas.save(args.output)
    values = [line[0] for line in found]
    print(f"lines that cannot be justified (not shown): {len(infinite)}")
    for limit in (100, 200, 500, 1000):
        print(f"lines worse than {limit}: {sum(1 for v in values if v > limit)} of {all_lines}")
    print("worst", [("inf" if math.isinf(v) else round(v)) for v in values[:10]])

if __name__ == "__main__":
    main()

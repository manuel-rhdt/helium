#!/usr/bin/env python3
"""Prints the file that `helium --dump-pagination FILE` writes as a table.

    helium --dump-pagination p.json in.html out.pdf && python3 tools/pagination_report.py p.json [--ways]

One line per page: the room that is left at its foot (in pixels), how it ends, what it breaks
(orphans, widows, an avoided break, an overflow), the paragraphs that it holds with the way that
was chosen for each (`*`) when they have more than one, and the cost that the paginator gave it.
With `--ways` the ways that the paragraphs had are listed too: the number of lines, the demerits
and the worst badness of each.
"""
import json, sys

def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    ways = "--ways" in sys.argv
    report = json.load(open(args[0]))
    print("tuning", report["tuning"], "| lines", report["lines"])
    print(f"{'page':>4} {'kind':<6} {'room':>7} {'ends':<8} {'flags':<12} {'lines':>5} {'way':<10} {'line room':>9} cost")
    for page in report["pages"]:
        flags = "".join(flag for flag, on in (("O", page["orphan_violation"]), ("W", page["widow_violation"]),
                                              ("X", page["overflow"]), ("A", page["ends_with"] == "Avoid")) if on)
        paragraphs = page["paragraphs"]
        lines = sum(p["lines"] for p in paragraphs)
        chosen = ",".join(f"{p['entity']}:{p['way']}" for p in paragraphs if p["way"])
        cost = page["cost"]
        print(f"{page['page']:>4} {page['kind']:<6} {page['room']:7.1f} {page['ends_with']:<8} {flags:<12} {lines:>5} "
              f"{chosen:<10} {page['line_room']:9.2f} slack {cost['slack']:.0f}")
        if ways:
            for p in paragraphs:
                if len(p["ways"]) > 1:
                    options = "  ".join(
                        ("*" if number == p["way"] else " ") + f"{w['lines']}l d={w['demerits']:.0f} b={w['worst_badness']:.0f}"
                        for number, w in enumerate(p["ways"]))
                    print(f"       paragraph {p['entity']}: {options}")

main()

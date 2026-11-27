#!/usr/bin/env python3
"""Writes a PDF that tests how a viewer and a display handle the colour spaces that Umbruch writes.

It does not use Umbruch or lemon_pdf: the PDF, the ICC profiles and the colour maths are written here, so a
bug in those cannot hide in the test. Open the file on the device and look at the seams.

    python3 tools/wide_gamut_test_pdf.py [output.pdf]

Pages:
 1  Lab /Range: every cell is split in two halves, the left one a Lab colour in a space with the range named on
    the left, the right one the same colour in an ICC space of the colour (the reference). A viewer that
    handles Lab right shows no seam wherever the colour is inside of the range.
 2  Neutrals: the same for greys, which show a tint at once if a* and b* are scaled or offset.
 3  Gamuts: ramps from 0 to 1 of each primary in each space; they clip on a screen that cannot show them.
 4  Probe: nested squares of Rec. 2020, Display P3 and sRGB primaries (a wide gamut screen with colour
    management shows rings), and a canary for DefaultRGB (a page defines it as Display P3, and a device colour
    shows which viewers honour that).
"""
import math
import struct
import sys

# ---------------------------------------------------------------- colour maths (CSS Color 4, in float64)

D50 = (0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585)
D65 = (0.3127 / 0.3290, 1.0, (1.0 - 0.3127 - 0.3290) / 0.3290)

BRADFORD_D65_D50 = [
    [1.0479298208405488, 0.022946793341019088, -0.05019222954313557],
    [0.029627815688159344, 0.990434484573249, -0.01707382502938514],
    [-0.009243058152591178, 0.015055144896577895, 0.7518742899580008],
]


def mul(m, v):
    return [sum(m[r][c] * v[c] for c in range(3)) for r in range(3)]


def primaries_matrix(xy, white):
    """RGB (linear) to XYZ from the chromaticities of the primaries and the white point."""
    cols = [[x / y, 1.0, (1 - x - y) / y] for x, y in xy]
    m = [[cols[c][r] for c in range(3)] for r in range(3)]
    inv = invert(m)
    s = mul(inv, list(white))
    return [[m[r][c] * s[c] for c in range(3)] for r in range(3)]


def invert(m):
    a, b, c = m[0]
    d, e, f = m[1]
    g, h, i = m[2]
    det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
    return [
        [(e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det],
        [(f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det],
        [(d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det],
    ]


def srgb_decode(v):
    return v / 12.92 if abs(v) <= 0.04045 else math.copysign(((abs(v) + 0.055) / 1.055) ** 2.4, v)


def gamma(g):
    return lambda v: math.copysign(abs(v) ** g, v)


def prophoto_decode(v):
    return v / 16 if abs(v) <= 16 / 512 else math.copysign(abs(v) ** 1.8, v)


LINEAR = lambda v: v

# name -> (matrix to XYZ relative to its own white, white, decode). Chromaticities from the specifications.
SPACES = {
    "sRGB": (primaries_matrix([(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)], D65), D65, srgb_decode),
    "Display P3": (primaries_matrix([(0.68, 0.32), (0.265, 0.69), (0.15, 0.06)], D65), D65, srgb_decode),
    "Adobe RGB": (primaries_matrix([(0.64, 0.33), (0.21, 0.71), (0.15, 0.06)], D65), D65, gamma(563 / 256)),
    "ProPhoto RGB": (primaries_matrix([(0.7347, 0.2653), (0.1596, 0.8404), (0.0366, 0.0001)], D50), D50, prophoto_decode),
    "Rec. 2020": (primaries_matrix([(0.708, 0.292), (0.170, 0.797), (0.131, 0.046)], D65), D65, gamma(2.4)),
    "sRGB linear": (primaries_matrix([(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)], D65), D65, LINEAR),
}


def to_xyz_d50(space, rgb):
    matrix, white, decode = SPACES[space]
    xyz = mul(matrix, [decode(c) for c in rgb])
    return xyz if white is D50 else mul(BRADFORD_D65_D50, xyz)


def to_lab(xyz):
    f = lambda t: t ** (1 / 3) if t > 216 / 24389 else (24389 / 27 * t + 16) / 116
    fx, fy, fz = (f(xyz[i] / D50[i]) for i in range(3))
    return 116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)


# ---------------------------------------------------------------- ICC profiles (version 2, matrix and curves)


def s15(v):
    return struct.pack(">i", round(v * 65536))


def icc_profile(space):
    matrix, white, decode = SPACES[space]
    columns = []
    for c in range(3):
        xyz = [matrix[r][c] for r in range(3)]
        columns.append(xyz if white is D50 else mul(BRADFORD_D65_D50, xyz))
    curve = b"curv\0\0\0\0" + struct.pack(">I", 1024) + b"".join(
        struct.pack(">H", round(max(0.0, min(1.0, decode(i / 1023))) * 65535)) for i in range(1024)
    )
    desc = space.encode()
    tags = {
        b"desc": b"desc\0\0\0\0" + struct.pack(">I", len(desc) + 1) + desc + b"\0" + struct.pack(">II", 0, 0) + struct.pack(">HB", 0, 0) + bytes(67),
        b"cprt": b"text\0\0\0\0Public domain\0",
        b"wtpt": b"XYZ \0\0\0\0" + b"".join(s15(v) for v in D50),
        b"rXYZ": b"XYZ \0\0\0\0" + b"".join(s15(v) for v in columns[0]),
        b"gXYZ": b"XYZ \0\0\0\0" + b"".join(s15(v) for v in columns[1]),
        b"bXYZ": b"XYZ \0\0\0\0" + b"".join(s15(v) for v in columns[2]),
        b"rTRC": curve,
    }
    names = list(tags)
    table = struct.pack(">I", len(names) + 2)
    offset = 128 + 4 + 12 * (len(names) + 2)
    body = b""
    placed = {}
    for name in names:
        data = tags[name]
        data += bytes(-len(data) % 4)
        placed[name] = (offset + len(body), len(tags[name]))
        body += data
        table += name + struct.pack(">II", *placed[name])
    # the three curves are the same data, stored once
    for name in (b"gTRC", b"bTRC"):
        table += name + struct.pack(">II", *placed[b"rTRC"])
    size = 128 + len(table) + len(body)
    header = struct.pack(">I4sI4s4s4s", size, b"\0\0\0\0", 0x02100000, b"mntr", b"RGB ", b"XYZ ")
    header += bytes(12) + b"acsp" + bytes(4 + 4 + 4 + 4 + 8) + struct.pack(">I", 0) + b"".join(s15(v) for v in D50) + bytes(4 + 16 + 28)
    assert len(header) == 128, len(header)
    return header + table + body


# ---------------------------------------------------------------- a small PDF writer


class Pdf:
    def __init__(self):
        self.objects = []

    def add(self, body):
        self.objects.append(body)
        return len(self.objects)

    def reserve(self):
        self.objects.append(None)
        return len(self.objects)

    def set(self, number, body):
        self.objects[number - 1] = body

    def stream(self, dictionary, data):
        return self.add(b"<< " + dictionary.encode() + b" /Length " + str(len(data)).encode() + b" >>\nstream\n" + data + b"\nendstream")

    def write(self, root, path):
        out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
        offsets = []
        for number, body in enumerate(self.objects, 1):
            offsets.append(len(out))
            out += str(number).encode() + b" 0 obj\n" + body + b"\nendobj\n"
        xref = len(out)
        out += b"xref\n0 " + str(len(self.objects) + 1).encode() + b"\n0000000000 65535 f \n"
        for offset in offsets:
            out += b"%010d 00000 n \n" % offset
        out += b"trailer\n<< /Size " + str(len(self.objects) + 1).encode() + b" /Root " + str(root).encode() + b" 0 R >>\nstartxref\n" + str(xref).encode() + b"\n%%EOF\n"
        with open(path, "wb") as f:
            f.write(out)


def num(v):
    return ("%.6f" % v).rstrip("0").rstrip(".") or "0"


class Page:
    """A page (or form) with named colour spaces and a content stream. y grows downwards here."""

    def __init__(self, pdf, width, height):
        self.pdf, self.width, self.height = pdf, width, height
        self.spaces = {}  # name -> PDF colour space array text
        self.ops = []
        self.xobjects = {}

    def space(self, name, text):
        self.spaces[name] = text

    def color(self, space, comps, stroke=False):
        self.ops.append("/%s %s %s %s" % (space, "CS" if stroke else "cs", " ".join(num(c) for c in comps), "SC" if stroke else "sc"))

    def device(self, rgb):
        self.ops.append("%s rg" % " ".join(num(c) for c in rgb))

    def rect(self, x, y, w, h):
        self.ops.append("%s %s %s %s re f" % (num(x), num(self.height - y - h), num(w), num(h)))

    def text(self, x, y, s, size=7, gray=0.0):
        s = s.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
        self.ops.append("BT /F1 %s Tf %s g %s %s Td (%s) Tj ET" % (num(size), num(gray), num(x), num(self.height - y - size), s))

    def draw(self, name, x, y):
        self.ops.append("q 1 0 0 1 %s %s cm /%s Do Q" % (num(x), num(-y), name))

    def resources(self, extra=""):
        spaces = " ".join("/%s %s" % (name, text) for name, text in self.spaces.items())
        xobjects = " ".join("/%s %d 0 R" % (name, ref) for name, ref in self.xobjects.items())
        return "<< /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> /ColorSpace << %s >> /XObject << %s >> >>" % (spaces, xobjects)

    def content(self):
        return ("\n".join(self.ops)).encode()


# ---------------------------------------------------------------- the document


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "wide-gamut-test.pdf"
    pdf = Pdf()
    catalog, pages = pdf.reserve(), pdf.reserve()

    profiles = {}

    def icc(space):
        if space not in profiles:
            data = icc_profile(space)
            profiles[space] = pdf.stream("/N 3 /Alternate /DeviceRGB", data)
        return "[/ICCBased %d 0 R]" % profiles[space]

    W, H = 420, 760
    page_refs = []

    def finish(page, extra_resources=""):
        content = pdf.stream("", page.content())
        ref = pdf.add(
            ("<< /Type /Page /Parent %d 0 R /MediaBox [0 0 %d %d] /Contents %d 0 R /Resources %s >>" % (pages, page.width, page.height, content, page.resources())).encode()
        )
        page_refs.append(ref)

    def title(p, heading, lines):
        p.text(20, 16, heading, 12)
        for i, line in enumerate(lines):
            p.text(20, 34 + i * 9, line, 6.5, 0.25)

    def lab_space(rng):
        r = rng
        rng_text = "[%s %s %s %s]" % (num(r[0]), num(r[1]), num(r[0]), num(r[1]))
        return "[/Lab << /WhitePoint [%s %s %s] /Range %s >>]" % (num(D50[0]), num(D50[1]), num(D50[2]), rng_text)

    ranges = [
        ("+-100", (-100, 100)),
        ("+-128", (-128, 128)),
        ("-128..127", (-128, 127)),
        ("+-150", (-150, 150)),
        ("+-200", (-200, 200)),
        ("+-256", (-256, 256)),
    ]

    # ---- page 1: Lab ranges against ICC references
    colors = [
        ("grey", "sRGB", (0.5, 0.5, 0.5)),
        ("sRGB red", "sRGB", (1, 0, 0)),
        ("sRGB green", "sRGB", (0, 1, 0)),
        ("sRGB blue", "sRGB", (0, 0, 1)),
        ("P3 red", "Display P3", (1, 0, 0)),
        ("P3 green", "Display P3", (0, 1, 0)),
        ("Adobe green", "Adobe RGB", (0, 1, 0)),
        ("2020 green", "Rec. 2020", (0, 1, 0)),
    ]
    p = Page(pdf, W, H)
    title(
        p,
        "1  Lab /Range against an ICC reference",
        [
            "Left half of every cell: CIELAB (D50) in a space with the range at the left. Right half: the same colour in an ICC",
            "space of the colour (named under the column). No seam = the viewer shows Lab right. A column whose |a*| or |b*|",
            "is beyond a range clips there by definition; compare it with the rows that hold it.",
        ],
    )
    for name, (_, rng) in zip(range(len(ranges)), ranges):
        pass
    for i, (label, rng) in enumerate(ranges):
        p.space("Lab%d" % i, lab_space(rng))
    for space in {c[1] for c in colors}:
        p.space("I" + space.replace(" ", "").replace(".", ""), icc(space))
    x0, y0, cw, ch = 82, 96, 40, 44
    for j, (label, space, rgb) in enumerate(colors):
        L, a, b = to_lab(to_xyz_d50(space, rgb))
        x = x0 + j * cw
        p.text(x + 2, y0 - 22, label, 5.5)
        p.text(x + 2, y0 - 14, "a %d" % round(a), 5.5, 0.35)
        p.text(x + 2, y0 - 7, "b %d" % round(b), 5.5, 0.35)
        for i, (rlabel, rng) in enumerate(ranges):
            y = y0 + i * ch
            p.color("Lab%d" % i, (L, a, b))
            p.rect(x, y, cw / 2, ch - 4)
            p.color("I" + space.replace(" ", "").replace(".", ""), rgb)
            p.rect(x + cw / 2, y, cw / 2, ch - 4)
    for i, (rlabel, rng) in enumerate(ranges):
        p.text(20, y0 + i * ch + 16, "Range " + rlabel, 7)
    y = y0 + len(ranges) * ch + 6
    for j, (label, space, rgb) in enumerate(colors):
        p.text(20, y + j * 9, "%s: %s %s" % (label, space, " ".join(num(c) for c in rgb)), 6.5, 0.25)
    finish(p)

    # ---- page 2: neutrals
    p = Page(pdf, W, H)
    title(
        p,
        "2  Neutrals",
        [
            "Lab greys (a* = b* = 0), left halves, against the same grey in ICC sRGB, right halves. A tint means that the",
            "viewer or the device scales or offsets a* and b*; -128..127 is the range that has no centre at 0.",
        ],
    )
    for i, (label, rng) in enumerate(ranges):
        p.space("Lab%d" % i, lab_space(rng))
    p.space("ISRGB", icc("sRGB"))
    levels = [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100]
    x0, y0, cw, ch = 70, 80, 30, 44
    for j, level in enumerate(levels):
        p.text(x0 + j * cw + 4, y0 - 9, "L %d" % level, 5.5, 0.35)
        grey = to_lab(to_xyz_d50("sRGB", [0.0] * 3))
        # the sRGB grey with this L* (D50 neutral): invert L* to Y, then to sRGB
        y_lin = ((level + 16) / 116) ** 3 if level > 8 else level / (24389 / 27)
        enc = 12.92 * y_lin if y_lin <= 0.0031308 else 1.055 * y_lin ** (1 / 2.4) - 0.055
        for i, (rlabel, rng) in enumerate(ranges):
            y = y0 + i * ch
            x = x0 + j * cw
            p.color("Lab%d" % i, (level, 0, 0))
            p.rect(x, y, cw / 2, ch - 4)
            p.color("ISRGB", (enc, enc, enc))
            p.rect(x + cw / 2, y, cw / 2, ch - 4)
    for i, (rlabel, rng) in enumerate(ranges):
        p.text(20, y0 + i * ch + 16, "Range " + rlabel, 6.5)
    finish(p)

    # ---- page 3: gamut ramps
    p = Page(pdf, W, H)
    title(
        p,
        "3  Ramps of the primaries in each space",
        [
            "Red, green and blue from 0 to 1 in 24 steps. On a screen (and in a viewer) that holds a space the ramp ends in the",
            "most saturated colour that it can show; where a ramp stops changing, the colour is clipped there.",
        ],
    )
    names = ["sRGB", "Display P3", "Adobe RGB", "ProPhoto RGB", "Rec. 2020", "sRGB linear"]
    for n in names:
        p.space("I" + n.replace(" ", "").replace(".", ""), icc(n))
    p.space("DevRGB", "/DeviceRGB")
    y = 70
    steps = 24
    p.text(20, y - 8, "DeviceRGB, page default (no DefaultRGB here)", 7)
    for c in range(3):
        for k in range(steps):
            rgb = [0, 0, 0]
            rgb[c] = k / (steps - 1)
            p.device(rgb)
            p.rect(20 + c * 128 + k * 5, y, 5, 34)
    y += 56
    for n in names:
        p.text(20, y - 8, n, 7)
        for c in range(3):
            for k in range(steps):
                rgb = [0, 0, 0]
                rgb[c] = k / (steps - 1)
                p.color("I" + n.replace(" ", "").replace(".", ""), rgb)
                p.rect(20 + c * 128 + k * 5, y, 5, 34)
        y += 56
    finish(p)

    # ---- page 4: probe and DefaultRGB canary
    p = Page(pdf, W, H)
    title(
        p,
        "4  Gamut probe and DefaultRGB",
        [
            "Probe: Rec. 2020 (outer), Display P3 (middle), sRGB (inner) primaries. A colour-managed wide-gamut screen shows",
            "three rings; an sRGB screen (or a viewer that clips to sRGB) shows one flat square.",
            "Canary: this page defines DefaultRGB as Display P3. A device colour that looks like the P3 swatch means the viewer honours",
            "DefaultRGB; one that looks like the sRGB swatch means it takes the device to be sRGB. The forms have resources of their own.",
        ],
    )
    for n in ("sRGB", "Display P3", "Rec. 2020"):
        p.space("I" + n.replace(" ", "").replace(".", ""), icc(n))
    p.space("DefaultRGB", icc("Display P3"))
    y = 90
    for label, prim in (("red", (1, 0, 0)), ("green", (0, 1, 0)), ("blue", (0, 0, 1))):
        pass
    for j, prim in enumerate([(1, 0, 0), (0, 1, 0), (0, 0, 1)]):
        x = 20 + j * 130
        p.color("IRec2020", prim)
        p.rect(x, y, 110, 110)
        p.color("IDisplayP3", prim)
        p.rect(x + 20, y + 20, 70, 70)
        p.color("IsRGB", prim)
        p.rect(x + 40, y + 40, 30, 30)
    # forms: one defines DefaultRGB as sRGB, one has no defaults
    def form(default):
        f = Page(pdf, 0, 0)
        f.height = 50
        if default:
            f.space("DefaultRGB", icc("sRGB"))
        return f

    cases = [("red", (1, 0, 0)), ("green", (0, 1, 0)), ("orange", (1, 0.5, 0))]
    p.text(20, 232, "Canary: [ICC sRGB] [ICC P3] [DeviceRGB on this page] [form, DefaultRGB sRGB] [form, none]", 6.5)
    y = 244
    for k, (label, rgb) in enumerate(cases):
        yy = y + k * 62
        p.color("IsRGB", rgb)
        p.rect(20, yy, 60, 50)
        p.color("IDisplayP3", rgb)
        p.rect(88, yy, 60, 50)
        p.device(rgb)
        p.rect(156, yy, 60, 50)
        for slot, default in enumerate((True, False)):
            f = form(default)
            f.device(rgb)
            f.ops.append("0 0 60 50 re f")
            data = f.content()
            ref = pdf.stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 60 50] /Group << /S /Transparency /CS /DeviceRGB >> /Resources "
                + ("<< /ColorSpace << /DefaultRGB %s >> >>" % icc("sRGB") if default else "<< >>"),
                data,
            )
            name = "F%d_%d" % (k, slot)
            p.xobjects[name] = ref
            p.ops.append("q 1 0 0 1 %s %s cm /%s Do Q" % (num(224 + slot * 68), num(H - yy - 50), name))
        p.text(364, yy + 20, label, 7)
    p.text(20, 440, "Nothing on this page is clipped by the PDF itself: any difference is the viewer or the display.", 6.5, 0.25)
    finish(p)

    pdf.set(pages, ("<< /Type /Pages /Kids [%s] /Count %d >>" % (" ".join("%d 0 R" % r for r in page_refs), len(page_refs))).encode())
    pdf.set(catalog, ("<< /Type /Catalog /Pages %d 0 R >>" % pages).encode())
    pdf.write(catalog, path)
    print("wrote", path)


if __name__ == "__main__":
    main()

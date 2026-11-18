#!/usr/bin/env python3
"""Computes the numbers and the figures of the letter and writes `index.html`.

    python3 gallery/article/generate.py        (needs numpy; takes about a minute)

The calculation is the one that the letter describes: the escape rate of an overdamped particle in the
double well U(x) = x^4/4 - x^2/2 from the exact mean first-passage time, Kramers' formula, and a Langevin
simulation as a check. The text of the letter is invented; the numbers in it are computed here.
"""
import math
import os
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))

# ---------------------------------------------------------------- the calculation

def U(x):
    return x**4 / 4 - x**2 / 2


def rate(D, zs_points=600_001, ys_points=20_001):
    """r = 1 / (2 tau), tau the mean first-passage time from the minimum at -1 to the barrier top at 0."""
    zs = np.linspace(-6.0, 0.0, zs_points)
    weight = np.exp(-(U(zs) - U(-1.0)) / D)
    cumulative = np.concatenate([[0.0], np.cumsum((weight[1:] + weight[:-1]) / 2 * np.diff(zs))])
    ys = np.linspace(-1.0, 0.0, ys_points)
    inner = np.interp(ys, zs, cumulative)
    outer = np.exp((U(ys) - U(-1.0)) / D)
    tau = np.trapezoid(outer * inner, ys) / D
    return 1.0 / (2.0 * tau)


def kramers(D):
    return math.sqrt(2.0) / (2.0 * math.pi) * math.exp(-1.0 / (4.0 * D))


def simulate(D, paths=4000, dt=1e-3, seed=7):
    """The mean first-passage time from -1 to 0 of dx = (x - x^3) dt + sqrt(2 D dt) N(0, 1), by Euler-Maruyama."""
    rng = np.random.default_rng(seed)
    x = np.full(paths, -1.0)
    t = np.zeros(paths)
    alive = np.ones(paths, bool)
    while alive.any():
        a = alive
        x[a] += (x[a] - x[a] ** 3) * dt + math.sqrt(2 * D * dt) * rng.standard_normal(a.sum())
        t[a] += dt
        alive &= ~(a & (x >= 0.0))
    tau, error = t.mean(), t.std() / math.sqrt(paths)
    return 1 / (2 * tau), error / tau / (2 * tau)


def bisect(f, low, high):
    for _ in range(60):
        mid = (low + high) / 2
        if f(low) * f(mid) <= 0:
            high = mid
        else:
            low = mid
    return (low + high) / 2


SIM_D = [0.15, 0.2, 0.3, 0.4, 0.5]
# (the simulations take most of the time; delete sim.json to run them again)
import json
cache = os.path.join(HERE, "sim.json")
if os.path.exists(cache):
    simulation = {float(k): tuple(v) for k, v in json.load(open(cache)).items()}
else:
    simulation = {D: simulate(D) for D in SIM_D}
    json.dump({str(k): v for k, v in simulation.items()}, open(cache, "w"))
ratio = lambda D: rate(D) / kramers(D)
d_min = min(np.linspace(0.04, 0.3, 261), key=ratio)
d_cross = bisect(lambda D: ratio(D) - 1.0, 0.15, 0.3)
# the leading correction: (r / r_K - 1) / D for small D
slopes = {D: (ratio(D) - 1) / D for D in (0.0025, 0.005, 0.0075)}
slope = np.polyfit(list(slopes), list(slopes.values()), 1)[1]  # extrapolated to D = 0
# convergence of the quadrature: halving the grids
grid_change = max(abs(rate(D, 300_001, 10_001) / rate(D) - 1) for D in (0.05, 0.1, 0.3))
err_c = lambda D: abs(kramers(D) * (1 - 1.5 * D) / rate(D) - 1)
scan = np.arange(0.004, 0.3, 0.002)
d_5 = next(D for D in scan if err_c(D) > 0.05)
d_1 = next(D for D in scan if err_c(D) > 0.01)
pulls = [abs(simulation[D][0] / rate(D) - 1) for D in SIM_D]

# ---------------------------------------------------------------- the figures (single column: 324 px)

W, H = 324, 200
FONT = 'font-family="TeX Gyre Termes, Times, serif"'


def axes(x0, x1, y0, y1, xticks, yticks, xlabel, ylabel, left=44, right=10, top=8, bottom=34, xfmt=str, yfmt=str):
    px = lambda x: left + (x - x0) / (x1 - x0) * (W - left - right)
    py = lambda y: H - bottom - (y - y0) / (y1 - y0) * (H - top - bottom)
    parts = [f'<rect x="{left}" y="{top}" width="{W - left - right}" height="{H - top - bottom}" fill="none" stroke="#000" stroke-width="0.8"/>']
    for x in xticks:
        parts.append(f'<line x1="{px(x):.1f}" y1="{H - bottom}" x2="{px(x):.1f}" y2="{H - bottom - 4}" stroke="#000" stroke-width="0.8"/>')
        parts.append(f'<text x="{px(x):.1f}" y="{H - bottom + 13}" font-size="10" text-anchor="middle" {FONT}>{xfmt(x)}</text>')
    for y in yticks:
        parts.append(f'<line x1="{left}" y1="{py(y):.1f}" x2="{left + 4}" y2="{py(y):.1f}" stroke="#000" stroke-width="0.8"/>')
        parts.append(f'<text x="{left - 5}" y="{py(y) + 3.5:.1f}" font-size="10" text-anchor="end" {FONT}>{yfmt(y)}</text>')
    parts.append(f'<text x="{(left + W - right) / 2:.1f}" y="{H - 5}" font-size="11" text-anchor="middle" font-style="italic" {FONT}>{xlabel}</text>')
    parts.append(f'<text x="11" y="{(top + H - bottom) / 2:.1f}" font-size="11" text-anchor="middle" font-style="italic" transform="rotate(-90 11 {(top + H - bottom) / 2:.1f})" {FONT}>{ylabel}</text>')
    return px, py, "".join(parts)


def polyline(points, **style):
    path = " ".join(f"{x:.1f},{y:.1f}" for x, y in points)
    attrs = " ".join(f'{k.replace("_", "-")}="{v}"' for k, v in style.items())
    return f'<polyline points="{path}" fill="none" {attrs}/>'


def figure_potential():
    px, py, frame = axes(-1.6, 1.6, -0.32, 0.48, [-1, 0, 1], [], "x", "U(x)", left=14, xfmt=lambda x: {-1: "−1", 0: "0", 1: "1"}[x])
    xs = np.linspace(-1.6, 1.6, 200)
    curve = polyline([(px(x), py(U(x))) for x in xs], stroke="#000", stroke_width="1.6")
    marks = "".join(f'<circle cx="{px(x):.1f}" cy="{py(U(x)):.1f}" r="3.2" fill="#000"/>' for x in (-1.0, 1.0))
    # the barrier height: U(0) - U(-1) = 1/4, a double arrow under the top of the barrier, where nothing
    # else is, with a dashed line that carries the level of the minimum over to it
    level = f'<line x1="{px(-1):.1f}" y1="{py(-0.25):.1f}" x2="{px(0):.1f}" y2="{py(-0.25):.1f}" stroke="#555" stroke-width="0.8" stroke-dasharray="3 2"/>'
    bx = px(0)
    height = (f'<line x1="{bx:.1f}" y1="{py(-0.25):.1f}" x2="{bx:.1f}" y2="{py(0.0):.1f}" stroke="#000" stroke-width="1"/>'
              f'<polygon points="{bx:.1f},{py(0.0):.1f} {bx - 2.5:.1f},{py(0.0) + 6:.1f} {bx + 2.5:.1f},{py(0.0) + 6:.1f}" fill="#000"/>'
              f'<polygon points="{bx:.1f},{py(-0.25):.1f} {bx - 2.5:.1f},{py(-0.25) - 6:.1f} {bx + 2.5:.1f},{py(-0.25) - 6:.1f}" fill="#000"/>'
              f'<text x="{bx + 7:.1f}" y="{py(-0.17) + 4:.1f}" font-size="10" {FONT}>ΔU = 1/4</text>')
    # x_a is above its minimum, inside the well; x_b above the top of the barrier
    labels = (f'<text x="{px(-1):.1f}" y="{py(-0.25) - 11:.1f}" font-size="11" text-anchor="middle" font-style="italic" {FONT}>x<tspan font-size="8" dy="2">a</tspan></text>'
              f'<text x="{px(0):.1f}" y="{py(0) - 7:.1f}" font-size="11" text-anchor="middle" font-style="italic" {FONT}>x<tspan font-size="8" dy="2">b</tspan></text>')
    return f'<svg viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" aria-label="The double well potential">{frame}{level}{curve}{marks}{height}{labels}</svg>'


def figure_rates():
    Ds = np.geomspace(0.04, 0.6, 70)
    exact = [rate(D) for D in Ds]
    px, py, frame = axes(0.0, 7.0, -4.2, -0.4, [0, 2, 4, 6], [-4, -3, -2, -1], "ΔU/D", '<tspan font-style="normal">log</tspan><tspan font-style="normal" font-size="8" dy="2">10</tspan><tspan dy="-2"> r</tspan>', xfmt=str, yfmt=lambda y: f"{y:.0f}".replace("-", "−"))
    x = lambda D: 1 / (4 * D)
    solid = polyline([(px(x(D)), py(math.log10(r))) for D, r in zip(Ds, exact)], stroke="#000", stroke_width="1.4")
    dashes = polyline([(px(x(D)), py(math.log10(kramers(D)))) for D in Ds], stroke="#000", stroke_width="1.2", stroke_dasharray="5 3")
    dots = "".join(f'<circle cx="{px(x(D)):.1f}" cy="{py(math.log10(simulation[D][0])):.1f}" r="3" fill="#fff" stroke="#000" stroke-width="1.1"/>' for D in SIM_D)
    key = (f'<line x1="{px(3.6):.1f}" y1="{py(-0.75):.1f}" x2="{px(4.5):.1f}" y2="{py(-0.75):.1f}" stroke="#000" stroke-width="1.4"/>'
           f'<text x="{px(4.65):.1f}" y="{py(-0.75) + 3.5:.1f}" font-size="10" {FONT}>exact</text>'
           f'<line x1="{px(3.6):.1f}" y1="{py(-1.1):.1f}" x2="{px(4.5):.1f}" y2="{py(-1.1):.1f}" stroke="#000" stroke-width="1.2" stroke-dasharray="5 3"/>'
           f'<text x="{px(4.65):.1f}" y="{py(-1.1) + 3.5:.1f}" font-size="10" {FONT}>Kramers</text>'
           f'<circle cx="{px(4.05):.1f}" cy="{py(-1.45):.1f}" r="3" fill="#fff" stroke="#000" stroke-width="1.1"/>'
           f'<text x="{px(4.65):.1f}" y="{py(-1.45) + 3.5:.1f}" font-size="10" {FONT}>simulation</text>')
    return f'<svg viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" aria-label="Escape rate against barrier height over noise strength">{frame}{solid}{dashes}{dots}{key}</svg>'


def figure_ratio():
    Ds = np.linspace(0.01, 0.5, 120)
    px, py, frame = axes(0.0, 0.5, 0.8, 1.5, [0, 0.1, 0.2, 0.3, 0.4, 0.5], [0.8, 1.0, 1.2, 1.4], "D", 'r / r<tspan font-size="8" dy="2">K</tspan>', xfmt=lambda x: f"{x:g}", yfmt=lambda y: f"{y:.1f}")
    one = f'<line x1="{px(0):.1f}" y1="{py(1):.1f}" x2="{px(0.5):.1f}" y2="{py(1):.1f}" stroke="#777" stroke-width="0.7" stroke-dasharray="2 2"/>'
    curve = polyline([(px(D), py(ratio(D))) for D in Ds], stroke="#000", stroke_width="1.4")
    line = polyline([(px(D), py(1 - 1.5 * D)) for D in np.linspace(0, 0.3, 40)], stroke="#000", stroke_width="1.1", stroke_dasharray="1.5 2.5")
    dots = "".join(f'<circle cx="{px(D):.1f}" cy="{py(simulation[D][0] / kramers(D)):.1f}" r="3" fill="#fff" stroke="#000" stroke-width="1.1"/>' for D in SIM_D)
    key = (f'<line x1="{px(0.05):.1f}" y1="{py(1.43):.1f}" x2="{px(0.1):.1f}" y2="{py(1.43):.1f}" stroke="#000" stroke-width="1.4"/>'
           f'<text x="{px(0.11):.1f}" y="{py(1.43) + 3.5:.1f}" font-size="10" {FONT}>exact</text>'
           f'<line x1="{px(0.05):.1f}" y1="{py(1.33):.1f}" x2="{px(0.1):.1f}" y2="{py(1.33):.1f}" stroke="#000" stroke-width="1.1" stroke-dasharray="1.5 2.5"/>'
           f'<text x="{px(0.11):.1f}" y="{py(1.33) + 3.5:.1f}" font-size="10" {FONT}>1 − 3D/2</text>'
           f'<circle cx="{px(0.075):.1f}" cy="{py(1.23):.1f}" r="3" fill="#fff" stroke="#000" stroke-width="1.1"/>'
           f'<text x="{px(0.11):.1f}" y="{py(1.23) + 3.5:.1f}" font-size="10" {FONT}>simulation</text>')
    return f'<svg viewBox="0 0 {W} {H}" width="{W}" height="{H}" role="img" aria-label="Ratio of the exact rate to the Kramers rate">{frame}{one}{curve}{line}{dots}{key}</svg>'


# ---------------------------------------------------------------- MathML

def mi(s, normal=False):
    return f'<mi mathvariant="normal">{s}</mi>' if normal else f"<mi>{s}</mi>"


mo = lambda s, stretchy=False: f'<mo stretchy="false">{s}</mo>' if s in "()" else f"<mo>{s}</mo>"
mn = lambda s: f"<mn>{s}</mn>"
row = lambda *p: "<mrow>" + "".join(p) + "</mrow>"
frac = lambda a, b: f"<mfrac>{a}{b}</mfrac>"
sup = lambda a, b: f"<msup>{a}{b}</msup>"
sub = lambda a, b: f"<msub>{a}{b}</msub>"
sqrt = lambda a: f"<msqrt>{a}</msqrt>"
d = mi("d", normal=True)
paren = lambda *p: mo("(") + "".join(p) + mo(")")
# fences that grow with a tall argument (a fraction): the operator dictionary makes them stretchy
bigparen = lambda *p: row("<mo>(</mo>", *p, "<mo>)</mo>")
sp = '<mspace width="0.2em"/>'
x, t, D_, y, z = mi("x"), mi("t"), mi("D"), mi("y"), mi("z")
Uf = lambda arg: mi("U") + paren(arg)
inline = lambda *p: "<math>" + "".join(p) + "</math>"
display = lambda *p: '<math display="block">' + "".join(p) + "</math>"

EQ1 = display(frac(row(d, x), row(d, t)), mo("="), mo("−"), sup(mi("U"), mo("′")), paren(x), mo("+"), sqrt(row(mn("2"), D_)), mi("ξ"), paren(t))
EQ2 = display(Uf(x), mo("="), frac(sup(x, mn("4")), mn("4")), mo("−"), frac(sup(x, mn("2")), mn("2")))
EQ3 = display(sub(mi("r"), mi("K")), mo("="), frac(sqrt(row(sup(mi("U"), mo("″")), paren(sub(x, mi("a"))), mo("|"), sup(mi("U"), mo("″")), paren(sub(x, mi("b"))), mo("|"))), row(mn("2"), mi("π"))),
               sup(mi("e"), row(mo("−"), mi("Δ"), mi("U"), mo("/"), D_)))
EQ3B = display(sub(mi("r"), mi("K")), mo("="), frac(sqrt(mn("2")), row(mn("2"), mi("π"))), sup(mi("e"), row(mo("−"), mn("1"), mo("/"), paren(mn("4"), D_))))
thin = '<mspace width="0.17em"/>'
EQ4 = display(mi("τ"), paren(sub(x, mn("0"))), mo("="), frac(mn("1"), D_), '<munderover><mo>∫</mo>' + sub(x, mn("0")) + mn("0") + "</munderover>", thin, d, y, thin, sup(mi("e"), row(Uf(y), mo("/"), D_)),
              thin, '<munderover><mo>∫</mo>' + row(mo("−"), mi("∞", True)) + y + "</munderover>", thin, d, z, thin, sup(mi("e"), row(mo("−"), Uf(z), mo("/"), D_)))
EQ5 = display(mi("r"), mo("="), frac(mn("1"), row(mn("2"), mi("τ"), paren(mn("−1")))))
EQ6 = display(frac(mi("r"), sub(mi("r"), mi("K"))), mo("="), mn("1"), mo("−"), frac(mn("3"), mn("2")), D_, mo("+"), mi("O"), paren(sup(D_, mn("2"))))

EQ7 = display(sub(mi("r"), mi("c")), mo("="), sub(mi("r"), mi("K")), bigparen(mn("1"), mo("−"), frac(mn("3"), mn("2")), D_))
CORR = inline(mo("⟨"), mi("ξ"), paren(t), mi("ξ"), paren(mi("t", False) + mo("′")), mo("⟩"), mo("="), mi("δ"), paren(t, mo("−"), mi("t") + mo("′")))
M = lambda *p: inline(*p)
M_U = M(mi("U"), paren(x))
M_D = M(D_)
M_dU = M(mi("Δ"), mi("U"), mo("="), mn("1"), mo("/"), mn("4"))
M_x = lambda sym: M(sym)

# ---------------------------------------------------------------- the table

def table_rows():
    rows = []
    for D in (0.05, 0.1, 0.15, 0.2, 0.3, 0.5):
        r, rk = rate(D), kramers(D)
        sim = f"{simulation[D][0]:.4f}" if D in simulation else "—"
        rows.append(f"<tr><td>{D:.2f}</td><td>{r:.4f}</td><td>{rk:.4f}</td><td>{r / rk:.3f}</td><td>{sim}</td></tr>")
    return "".join(rows)


def table2_rows():
    rows = []
    for D in (0.01, 0.02, 0.05, 0.1, 0.15):
        r, rk = rate(D), kramers(D)
        rows.append(f"<tr><td>{D:.2f}</td><td>{1 / (4 * D):.1f}</td><td>{100 * (rk / r - 1):+.1f}</td><td>{100 * (rk * (1 - 1.5 * D) / r - 1):+.1f}</td></tr>".replace("-", "−"))
    return "".join(rows)


# ---------------------------------------------------------------- the document

TEMPLATE = open(os.path.join(HERE, "template.html")).read()
numbers = {
    "FIG1": figure_potential(),
    "FIG2": figure_rates(),
    "FIG3": figure_ratio(),
    "TABLE": table_rows(),
    "EQ1": EQ1, "EQ2": EQ2, "EQ3": EQ3, "EQ3B": EQ3B, "EQ4": EQ4, "EQ5": EQ5, "EQ6": EQ6,
    "M_U": M_U, "M_D": M_D, "M_dU": M_dU,
    "D_MIN": f"{d_min:.2f}", "R_MIN": f"{ratio(d_min):.3f}", "OVER_MAX": f"{100 * (1 / ratio(d_min) - 1):.0f}",
    "D_CROSS": f"{d_cross:.2f}", "R_HALF": f"{ratio(0.5):.2f}", "UNDER_HALF": f"{100 * (1 - 1 / ratio(0.5)):.0f}",
    "SLOPE": f"{slope:.2f}".replace("-", "−"),
    "GRID": f"{grid_change:.0e}".replace("e-0", "×10<sup>−").replace("e-", "×10<sup>−") + "</sup>",
    "SIM_AGREE": f"{100 * max(pulls):.0f}",
    "DU_OVER_D_LO": f"{1 / (4 * 0.6):.1f}",
    "DU_D_MIN": f"{1 / (4 * d_min):.1f}", "DU_D_CROSS": f"{1 / (4 * d_cross):.1f}",
    "EQ7": EQ7, "CORR": CORR, "UPP": inline(sup(mi("U"), mo("″"))), "TABLE2": table2_rows(), "D_5": f"{d_5:.2f}", "D_1": f"{d_1:.3f}".rstrip("0"),
}
for name, value in numbers.items():
    TEMPLATE = TEMPLATE.replace(f"@@{name}@@", value)
open(os.path.join(HERE, "index.html"), "w").write(TEMPLATE)
print({k: v for k, v in numbers.items() if len(v) < 40})

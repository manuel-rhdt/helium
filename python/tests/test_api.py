import io
import os
import subprocess
import sys

import pytest

import umbruch

HTML = "<html><body><h1 id='t'>Hello</h1><p>wide</p></body></html>"


@pytest.fixture(scope="module")
def server():
    if "UMBRUCH" not in os.environ and umbruch.find_umbruch() is None:
        pytest.skip("no umbruch executable: set $UMBRUCH")
    with umbruch.Server(system_fonts=False) as server:
        yield server


def test_write_pdf_returns_bytes(server):
    pdf = umbruch.HTML(HTML, server=server).write_pdf()
    assert pdf.startswith(b"%PDF-")


def test_write_pdf_to_a_path_and_a_file_object(server, tmp_path):
    document = umbruch.HTML(HTML, server=server)
    assert document.write_pdf(tmp_path / "a.pdf") is None
    assert (tmp_path / "a.pdf").read_bytes().startswith(b"%PDF-")
    buffer = io.BytesIO()
    document.write_pdf(buffer)
    assert buffer.getvalue().startswith(b"%PDF-")


def test_render_reports_pages_and_size(server):
    rendered = umbruch.HTML("<p style='break-after: page'>a</p><p>b</p>", server=server).render()
    assert rendered.pages == 2
    assert rendered.page_size_pt == pytest.approx((595.28, 841.89), abs=1)
    assert isinstance(rendered.warnings, list)


def test_page_size_comes_from_the_at_page_rule_and_crop(server):
    a = umbruch.HTML("<style>@page { size: 400px 300px }</style><p>x</p>", server=server).render()
    assert a.page_size_pt == pytest.approx((300, 225), abs=1)
    b = umbruch.HTML("<div style='width:100px;height:50px'></div>", server=server).render(crop=True)
    assert b.pages == 1 and b.page_size_pt[0] < 300


def test_pdfa_and_untagged_options(server):
    pdf = umbruch.HTML(HTML, server=server).write_pdf(pdfa="2b", compress=False)
    assert b"pdfaid" in pdf
    untagged = umbruch.HTML(HTML, server=server).write_pdf(tags=False, compress=False)
    assert b"/StructTreeRoot" not in untagged


def test_filename_sets_the_base_directory_and_kind(server, tmp_path):
    (tmp_path / "pic.svg").write_text(
        "<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><rect width='10' height='10' fill='red'/></svg>"
    )
    (tmp_path / "doc.html").write_text("<img src='pic.svg' width='10' height='10'>")
    rendered = umbruch.HTML(filename=tmp_path / "doc.html", server=server).render()
    assert rendered.pages == 1
    assert not [w for w in rendered.warnings if "cannot load" in w], rendered.warnings
    missing = umbruch.HTML("<img src='nope.svg'>", base_dir=tmp_path, server=server).render()
    assert any("cannot load" in w for w in missing.warnings)
    svg = umbruch.HTML(filename=tmp_path / "pic.svg", server=server)
    assert svg.kind == "xml"
    assert svg.render().pages == 1


def test_wide_gamut_colors_reach_the_pdf(server):
    pdf = umbruch.HTML(
        "<svg width='20' height='20'><rect width='20' height='20' fill='color(display-p3 1 0 0)'/></svg>", server=server
    ).write_pdf(compress=False)
    assert b"/ICCBased" in pdf


def test_layout_gives_boxes(server):
    result = umbruch.HTML(HTML, server=server).layout(["t"])
    assert result["missing_ids"] == []
    assert "t" in result["boxes"]


def test_measure(server):
    metrics = server.measure([{"text": "Hello"}, {"text": ""}])
    assert metrics[0]["width"] > 0


def test_errors_carry_the_protocol_code(server):
    with pytest.raises(umbruch.Error) as info:
        umbruch.HTML("<p>x</p>", server=server).render(pdfa="9z")
    assert info.value.code == "bad_request"
    # the server is still usable afterwards
    assert umbruch.HTML(HTML, server=server).write_pdf()


def test_a_server_that_died_is_started_again(server):
    server.render("<p>x</p>")
    server._proc.kill()
    server._proc.wait()
    assert server.render("<p>y</p>")[1].startswith(b"%PDF-")


def test_exactly_one_source_is_required():
    with pytest.raises(TypeError):
        umbruch.HTML()
    with pytest.raises(TypeError):
        umbruch.HTML("x", filename="y.html")


def test_a_missing_executable_is_an_error():
    with pytest.raises(umbruch.Error) as info:
        umbruch.Server("/nonexistent/umbruch").measure([])
    assert info.value.code == "server"

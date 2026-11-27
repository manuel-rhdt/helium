"""umbruch: an HTML/CSS/MathML/SVG to PDF renderer.

::

    import umbruch

    umbruch.HTML("<h1>Hello</h1>").write_pdf("hello.pdf")
    pdf = umbruch.HTML(filename="report.html").write_pdf()      # bytes
    rendered = umbruch.HTML("<p>...</p>").render(pdfa="2b")
    rendered.pages, rendered.warnings

The documents are rendered by one long-lived ``umbruch --serve`` process that starts with the first
request (see :class:`Server` for the fonts and other settings, and to run several at once).
"""
import os
import shutil
import sysconfig
from pathlib import Path

from ._server import Error, Server

__all__ = ["HTML", "Rendered", "Server", "Error", "default_server", "find_umbruch"]

_EXE = "umbruch.exe" if os.name == "nt" else "umbruch"


def find_umbruch():
    """Path of the umbruch executable installed with this package, or None.

    Looks in the scripts directory of the running environment (also the user scheme and,
    for editable setups, ``PATH``) rather than assuming that the environment is activated.
    """
    candidates = [sysconfig.get_path("scripts")]
    for scheme in (f"{os.name}_user", "posix_user", "nt_user"):
        try:
            candidates.append(sysconfig.get_path("scripts", scheme))
        except KeyError:
            pass
    for d in candidates:
        if d and (Path(d) / _EXE).is_file():
            return str(Path(d) / _EXE)
    return shutil.which("umbruch")


_default = None


def default_server():
    """The server that documents are rendered with unless they are given another."""
    global _default
    if _default is None:
        _default = Server()
    return _default


class Rendered:
    """A rendered document: the PDF (``pdf``, bytes), the number of ``pages``, the size of the
    first page in points (``page_size_pt``, or None) and the ``warnings`` that came up."""

    def __init__(self, pdf, pages, page_size_pt, warnings):
        self.pdf = pdf
        self.pages = pages
        self.page_size_pt = tuple(page_size_pt) if page_size_pt else None
        self.warnings = list(warnings)

    def write_pdf(self, target=None):
        """Writes the PDF to a path or a binary file object, or returns it as bytes without one."""
        return _write(self.pdf, target)

    def __repr__(self):
        return f"<umbruch.Rendered {self.pages} page(s), {len(self.pdf)} bytes>"


class HTML:
    """A document to render, from a ``string`` or a ``filename``.

    ``base_dir`` is the directory that relative paths of images are taken from: the directory of
    the file, or the current directory for a string. A file with the extension ``.svg``, ``.xhtml``,
    ``.xht`` or ``.xml`` is read as XML (well-formed, with namespaces), anything else with the HTML
    parser; ``xml=True`` or ``xml=False`` decides it. ``server`` renders with another :class:`Server`.
    """

    def __init__(self, string=None, *, filename=None, base_dir=None, xml=None, server=None):
        if (string is None) == (filename is None):
            raise TypeError("give exactly one of `string` and `filename`")
        if filename is not None:
            filename = Path(filename)
            string = filename.read_text(encoding="utf-8", errors="replace")
            if base_dir is None:
                base_dir = filename.resolve().parent
            if xml is None:
                xml = filename.suffix.lower() in (".svg", ".xhtml", ".xht", ".xml")
        elif isinstance(string, bytes):
            string = string.decode("utf-8", "replace")
        self.string = string
        self.base_dir = Path(base_dir) if base_dir is not None else Path.cwd()
        self.kind = "xml" if xml else "html"
        self.server = server

    def render(self, *, crop=False, pdfa=None, tags=True, compress=True):
        """Renders the document to a :class:`Rendered`.

        ``crop`` makes one page that is as large as the content. The size of the pages is the one of
        the ``@page`` rules of the document (``@page { size: A5 landscape; margin: 2cm }``), A4 if
        there is none. ``pdfa`` is a level (``"1b"``, ``"2a"``, ``"2b"``, ``"2u"``, ``"3a"``, ``"3b"`` or
        ``"3u"``); ``tags=False`` leaves out the structure tree; ``compress=False`` writes
        streams uncompressed. Raises :class:`Error` if the document cannot be rendered.
        """
        options = {}
        if crop:
            options["crop"] = True
        if pdfa is not None:
            options["pdfa"] = str(pdfa)
        if not tags:
            options["no_tags"] = True
        if not compress:
            options["no_compression"] = True
        server = self.server or default_server()
        result, pdf = server.render(self.string, kind=self.kind, base_dir=self.base_dir, **options)
        return Rendered(pdf, result.get("pages", 0), result.get("page_size_pt"), result.get("warnings", ()))

    def write_pdf(self, target=None, **options):
        """Renders (see :meth:`render`) and writes the PDF to a path or a binary file object, or
        returns it as bytes if there is no target."""
        return self.render(**options).write_pdf(target)

    def layout(self, ids=(), *, dump=False, crop=False):
        """The boxes of the elements with these ids (CSS pixels, by page), as the server gives them."""
        options = {"crop": True} if crop else {}
        server = self.server or default_server()
        return server.layout(self.string, kind=self.kind, base_dir=self.base_dir, ids=ids, dump=dump, **options)


def _write(pdf, target):
    if target is None:
        return pdf
    if hasattr(target, "write"):
        target.write(pdf)
    else:
        with open(os.fspath(target), "wb") as file:
            file.write(pdf)
    return None

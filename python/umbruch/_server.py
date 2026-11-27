"""A client of ``umbruch --serve``: one long-lived process that lays out and renders documents.

The protocol is described in the documentation of ``umbruch::server`` (version 1): requests and
responses are JSON with ``Content-Length`` framing on stdio, and a rendered PDF follows its
response as raw bytes.
"""
import atexit
import json
import os
import subprocess
import threading

__all__ = ["Error", "Server"]

PROTOCOL = 1


class Error(Exception):
    """A request that umbruch answered with an error, or a server that cannot be reached.

    ``code`` is the code of the protocol (``parse_error``, ``layout_error``, ``render_error``,
    ``bad_request``, ...), or ``"server"`` if the process died or spoke something else.
    """

    def __init__(self, code, message):
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message


class Server:
    """One ``umbruch --serve`` process, started when it is first needed.

    A process that exits (a panic, or the time limit of 120 s per request) is started again, and
    the request is repeated once. Requests are answered one at a time: the server is safe to share
    between threads, which then wait for each other. Use one ``Server`` per worker to render in
    parallel.

    ``executable`` defaults to the one that the ``umbruch`` package installed. Set
    ``system_fonts=False`` (only the fonts built into umbruch, so the output does not depend on the
    machine), ``font_dirs`` (more directories of fonts) and ``resource_root`` (the directory that
    documents may take files from) to configure it; they are fixed for the life of the process.
    """

    def __init__(self, executable=None, *, system_fonts=True, font_dirs=(), resource_root=None):
        self.executable = str(executable) if executable is not None else _executable()
        args = []
        if not system_fonts:
            args.append("--no-system-fonts")
        for directory in font_dirs:
            args += ["--font-dir", os.fspath(directory)]
        if resource_root is not None:
            args += ["--resource-root", os.fspath(resource_root)]
        self.args = args
        self._proc = None
        self._lock = threading.RLock()
        self._next_id = 0
        self.version = None
        self.capabilities = frozenset()
        atexit.register(self.close)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()

    # -- requests ------------------------------------------------------------------------------

    def render(self, source, *, kind="html", base_dir=None, **options):
        """Renders a document. Returns ``(result, pdf)``: the ``pages``, ``page_size_pt`` and
        ``warnings`` of the result, and the PDF as bytes.

        ``options``: ``crop``, ``page`` (``{"width", "height"}`` in CSS pixels), ``no_compression``,
        ``no_tags`` and ``pdfa`` (the level, like ``"2b"``).
        """
        return self._request("render", {"document": _document(source, kind, base_dir), "options": options})

    def layout(self, source, *, kind="html", base_dir=None, ids=(), dump=False, **options):
        """The boxes of the elements with these ``ids`` (in CSS pixels, by page) and, with
        ``dump``, the whole layout tree. Returns the result as a dict (``boxes``, ``missing_ids``,
        ``dump``, ``warnings``)."""
        query = {"ids": list(ids), "dump": dump}
        result, _ = self._request("layout", {"document": _document(source, kind, base_dir), "options": options, "query": query})
        return result

    def measure(self, probes):
        """The size of pieces of text or markup, each ``{"text" | "markup", "css"}``: a list with
        ``width``, ``ascent``, ``descent``, ``height``, ``ink`` and ``missing_glyphs`` for each (or
        an ``error``)."""
        result, _ = self._request("measure", {"probes": list(probes)})
        return result["metrics"]

    def close(self):
        """Stops the process (it is started again by the next request)."""
        with self._lock:
            proc, self._proc = self._proc, None
            if proc is None:
                return
            try:
                self._send(proc, "shutdown", {})
                proc.stdin.close()
                proc.wait(timeout=5)
            except Exception:
                proc.kill()
                proc.wait()
            finally:
                for stream in (proc.stdin, proc.stdout):
                    try:
                        stream.close()
                    except Exception:
                        pass

    # -- process and framing -------------------------------------------------------------------

    def _request(self, op, params):
        with self._lock:
            for attempt in (0, 1):
                if self._proc is None or self._proc.poll() is not None:
                    self._start()
                try:
                    return self._exchange(self._proc, op, params)
                except (EOFError, BrokenPipeError, OSError) as error:
                    self._discard()
                    if attempt:
                        raise Error("server", f"the umbruch process stopped: {error}") from error

    def _start(self):
        try:
            self._proc = subprocess.Popen(
                [self.executable, *self.args, "--serve"], stdin=subprocess.PIPE, stdout=subprocess.PIPE
            )
        except OSError as error:
            raise Error("server", f"cannot start {self.executable!r}: {error}") from error
        try:
            hello, _ = self._exchange(self._proc, "hello", {"protocol": PROTOCOL})
        except Exception:
            self._discard()
            raise
        if hello.get("protocol") != PROTOCOL:
            self._discard()
            raise Error("server", f"umbruch speaks protocol {hello.get('protocol')}, expected {PROTOCOL}")
        self.version = hello.get("umbruch")
        self.capabilities = frozenset(hello.get("capabilities", ()))

    def _discard(self):
        proc, self._proc = self._proc, None
        if proc is not None:
            try:
                proc.kill()
            except OSError:
                pass
            proc.wait()
            for stream in (proc.stdin, proc.stdout):
                try:
                    stream.close()
                except Exception:
                    pass

    def _send(self, proc, op, params):
        self._next_id += 1
        body = json.dumps({"id": self._next_id, "op": op, "params": params}, ensure_ascii=False).encode("utf-8")
        proc.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
        proc.stdin.flush()
        return self._next_id

    @staticmethod
    def _read_exact(proc, n):
        data = proc.stdout.read(n)
        if len(data) != n:
            raise EOFError("the stream was closed")
        return data

    def _exchange(self, proc, op, params):
        request_id = self._send(proc, op, params)
        length = None
        while True:
            line = proc.stdout.readline()
            if not line:
                raise EOFError("the stream was closed")
            line = line.strip()
            if not line:
                break
            name, _, value = line.decode("ascii", "replace").partition(":")
            if name.lower() == "content-length":
                length = int(value)
        if length is None:
            raise Error("server", "a response without Content-Length")
        reply = json.loads(self._read_exact(proc, length))
        if reply.get("id") != request_id:
            self._discard()
            raise Error("server", f"umbruch answered request {reply.get('id')}, expected {request_id}")
        if not reply.get("ok"):
            error = reply.get("error", {})
            raise Error(error.get("code", "server"), error.get("message", "the request failed"))
        result = reply["result"]
        attachment = self._read_exact(proc, result["attachment_length"]) if result.get("attachment_length") else b""
        return result, attachment


def _document(source, kind, base_dir):
    document = {"source": source, "kind": kind}
    if base_dir is not None:
        document["base_dir"] = os.fspath(base_dir)
    return document


def _executable():
    from . import find_umbruch

    path = os.environ.get("UMBRUCH") or find_umbruch()
    if path is None:
        raise Error("server", "the umbruch executable was not found (install the `umbruch` package or set $UMBRUCH)")
    return path

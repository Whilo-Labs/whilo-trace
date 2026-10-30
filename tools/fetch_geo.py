"""Downloads the places database of the route map: DB-IP City Lite and ASN Lite of one month, unpacked and checked,
with manifest.json and NOTICE.txt next to them. The release workflow runs it once per release and bundles the folder
(app/tauri.geo.conf.json); a developer can point WHILO_TRACE_GEO_DIR at it.

Usage: python tools/fetch_geo.py [--month YYYY-MM] [--out .target/geo]
       python tools/fetch_geo.py --self-test

Without --month it takes the current month (UTC), or the month before when DB-IP has not published the current one
yet (HTTP 404). Nothing is written in place: every file goes under a temporary name and is renamed only after both
databases are checked, because a running program may have the old files mapped. The databases are DB-IP's, under
CC BY 4.0 (NOTICE.txt). --self-test checks the MMDB metadata reader, the months and whole runs on tiny databases
made in memory, without the network. Pure standard library, so it runs anywhere.
"""
import argparse
import datetime
import gzip
import hashlib
import http.client
import io
import json
import os
import re
import struct
import sys
import tempfile
import urllib.error
import urllib.request
import zlib
from pathlib import Path

BASE = "https://download.db-ip.com/free/"
USER_AGENT = "Whilo-Trace-build (+https://github.com/Whilo-Labs/whilo-trace)"

# What the program reads in its bundled folder (crates/geo/src/files.rs). Per database: the key in the manifest,
# DB-IP's file name stem, the bundled file name and the word its database_type must contain. ASN comes first: it is
# the small one, so a month that is not out yet costs little.
DATABASES = (
    ("asn", "dbip-asn-lite", "dbip-asn-lite.mmdb", "ASN"),
    ("city", "dbip-city-lite", "dbip-city-lite.mmdb", "City"),
)
MANIFEST = "manifest.json"
NOTICE = "NOTICE.txt"

# The metadata of a MaxMind DB file follows the last copy of this marker, within the last 128 KiB of the file.
MARKER = b"\xab\xcd\xefMaxMind.com"
METADATA_MAX = 128 * 1024

# The app's limits for an update (crates/geo/src/update.rs), far above the real files.
MAX_GZ = 512 << 20
MAX_DB = 1 << 30
CHUNK = 1 << 20

# Bytes an unsigned integer of each MMDB type may have: uint16, uint32, uint64, uint128.
UINT_BYTES = {5: 2, 6: 4, 9: 8, 10: 16}


class FetchError(Exception):
    """A run that cannot finish; the message says why."""


class NotPublished(FetchError):
    """DB-IP has no file of that month (HTTP 404)."""


class MmdbError(FetchError):
    """A file that is not the MaxMind DB database it should be."""


def is_month(s):
    """A month the way DB-IP writes it in file names: 2026-10."""
    return re.fullmatch(r"[0-9]{4}-(0[1-9]|1[0-2])", s) is not None


def current_month(now=None):
    """The month of `now` (an aware datetime, default this moment) in UTC."""
    now = now or datetime.datetime.now(datetime.timezone.utc)
    return now.astimezone(datetime.timezone.utc).strftime("%Y-%m")


def prev_month(month):
    year, mon = int(month[:4]), int(month[5:])
    return "%04d-%02d" % ((year, mon - 1) if mon > 1 else (year - 1, 12))


def decode(buf, pos=0):
    """One value of the MaxMind DB data format at `pos` in `buf`: (value, position after it). Maps, arrays, strings,
    bytes, doubles, floats, booleans, signed and unsigned integers and pointers; pointers count from the start of
    `buf`, which for the metadata is the byte after the marker."""
    left = [100000]  # values: a few pointers could make a small file decode into a huge structure

    def take(pos, n):
        if pos + n > len(buf):
            raise MmdbError("the metadata is cut short")
        return buf[pos:pos + n]

    def value(pos, depth):
        left[0] -= 1
        if left[0] < 0 or depth > 32:
            raise MmdbError("the metadata is too large or nests too deep")
        ctrl = take(pos, 1)[0]
        pos += 1
        kind = ctrl >> 5
        if kind == 1:
            # Pointer 001SSVVV: SS + 1 more bytes, with the three V bits in front of them unless SS is 3.
            size = (ctrl >> 3) & 3
            raw = int.from_bytes(take(pos, size + 1), "big")
            pos += size + 1
            if size < 3:
                raw = ((ctrl & 7) << (8 * (size + 1)) | raw) + (0, 2048, 526336)[size]
            return value(raw, depth + 1)[0], pos
        if kind == 0:
            # Extended type: the next byte is the type minus 7.
            kind = 7 + take(pos, 1)[0]
            pos += 1
        size = ctrl & 31
        if size >= 29:
            n = size - 28
            size = (29, 285, 65821)[n - 1] + int.from_bytes(take(pos, n), "big")
            pos += n
        if kind == 7:
            out = {}
            for _ in range(size):
                key, pos = value(pos, depth + 1)
                if not isinstance(key, str):
                    raise MmdbError("a map key that is not a string")
                out[key], pos = value(pos, depth + 1)
            return out, pos
        if kind == 11:
            items = []
            for _ in range(size):
                item, pos = value(pos, depth + 1)
                items.append(item)
            return items, pos
        if kind == 14:
            if size > 1:
                raise MmdbError("a boolean of %d" % size)
            return size == 1, pos
        raw = take(pos, size)
        pos += size
        if kind == 2:
            try:
                return raw.decode("utf-8"), pos
            except UnicodeDecodeError:
                raise MmdbError("a string that is not UTF-8") from None
        if kind == 4:
            return bytes(raw), pos
        if kind in (3, 15):
            width, fmt = (8, ">d") if kind == 3 else (4, ">f")
            if size != width:
                raise MmdbError("a floating point number of %d bytes" % size)
            return struct.unpack(fmt, raw)[0], pos
        if kind in UINT_BYTES and size <= UINT_BYTES[kind]:
            return int.from_bytes(raw, "big"), pos
        if kind == 8 and size <= 4:
            return int.from_bytes(raw, "big", signed=size == 4), pos
        raise MmdbError("data type %d of %d bytes in the metadata" % (kind, size))

    return value(pos, 0)


def read_metadata(tail):
    """The metadata map of a MaxMind DB file, from its last bytes (METADATA_MAX of them are enough)."""
    at = tail.rfind(MARKER)
    if at < 0:
        raise MmdbError("no MaxMind DB metadata marker")
    meta = decode(tail[at + len(MARKER):])[0]
    if not isinstance(meta, dict):
        raise MmdbError("the metadata is not a map")
    return meta


def check_metadata(meta, kind, file_size):
    """Makes sure `meta` describes a MaxMind DB 2.x file of `file_size` bytes whose database_type names `kind`
    ("City" or "ASN", in any case, as the program checks it); returns the type."""
    ty = meta.get("database_type")
    if not isinstance(ty, str) or kind.lower() not in ty.lower():
        raise MmdbError("the database type is %r, expected %s" % (ty, kind))
    if meta.get("binary_format_major_version") != 2:
        raise MmdbError("format version %r, expected 2" % meta.get("binary_format_major_version"))
    nodes, record, ip = meta.get("node_count"), meta.get("record_size"), meta.get("ip_version")
    if ip not in (4, 6) or record not in (24, 28, 32) or not isinstance(nodes, int) or nodes < 1:
        raise MmdbError("no search tree: ip_version %r, record_size %r, node_count %r" % (ip, record, nodes))
    # The search tree (two records per node) and the 16 zero bytes after it come before the metadata.
    if nodes * record // 4 + 16 > file_size:
        raise MmdbError("a search tree of %d nodes does not fit in %d bytes" % (nodes, file_size))
    if not isinstance(meta.get("build_epoch"), int):
        raise MmdbError("no build_epoch")
    return ty


def check_file(path, kind):
    """The database_type of the MaxMind DB file at `path`, after check_metadata."""
    size = os.path.getsize(path)
    with open(path, "rb") as f:
        f.seek(max(0, size - METADATA_MAX))
        tail = f.read()
    return check_metadata(read_metadata(tail), kind, size)


class _Limited:
    """A response body for gzip to read, cut off after `limit` bytes."""

    def __init__(self, body, limit, name):
        self.body, self.left, self.limit, self.name = body, limit, limit, name

    def read(self, n=-1):
        data = self.body.read(n if n is not None and n >= 0 else self.left + 1)
        self.left -= len(data)
        if self.left < 0:
            raise FetchError("%s: larger than %d bytes" % (self.name, self.limit))
        return data


def download(url, dest, opener):
    """Streams the gzip file at `url` into `dest`, unpacked; returns the size and SHA-1 of the unpacked file. HTTP 404
    raises NotPublished."""
    name = url.rsplit("/", 1)[-1]
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        response = opener(request, timeout=60)
    except urllib.error.HTTPError as e:
        e.close()
        if e.code == 404:
            raise NotPublished("%s: HTTP 404" % name) from None
        raise FetchError("%s: HTTP %d" % (name, e.code)) from None
    except OSError as e:
        raise FetchError("%s: %s" % (name, getattr(e, "reason", e))) from None
    sha1, size = hashlib.sha1(), 0
    try:
        with response, open(dest, "wb") as out:
            with gzip.GzipFile(fileobj=_Limited(response, MAX_GZ, name), mode="rb") as gz:
                while True:
                    chunk = gz.read(CHUNK)
                    if not chunk:
                        break
                    size += len(chunk)
                    if size > MAX_DB:
                        raise FetchError("%s: unpacks to more than %d bytes" % (name, MAX_DB))
                    sha1.update(chunk)
                    out.write(chunk)
            out.flush()
            os.fsync(out.fileno())
    except (OSError, EOFError, zlib.error, http.client.HTTPException) as e:
        raise FetchError("%s: %s" % (name, e)) from None
    return size, sha1.hexdigest()


def notice(month, base):
    """NOTICE.txt: the credit and the license DB-IP asks for, next to the files."""
    return (
        "Places database of Whilo Trace\n"
        "\n"
        "dbip-city-lite.mmdb: DB-IP IP to City Lite, %(m)s\n"
        "dbip-asn-lite.mmdb: DB-IP IP to ASN Lite, %(m)s\n"
        "\n"
        "IP Geolocation by DB-IP: https://db-ip.com\n"
        "\n"
        "Both databases are made by DB-IP.com and licensed under the Creative Commons Attribution 4.0\n"
        "International License (CC BY 4.0): https://creativecommons.org/licenses/by/4.0/\n"
        "These files are under CC BY 4.0, not under the MIT or Apache-2.0 license of Whilo Trace.\n"
        "\n"
        "They are not modified apart from decompression: they are dbip-city-lite-%(m)s.mmdb.gz\n"
        "and dbip-asn-lite-%(m)s.mmdb.gz from %(base)s, unpacked and\n"
        "saved under the names above. As section 5 of CC BY 4.0 says, they come as-is, without\n"
        "warranties.\n"
    ) % {"m": month, "base": base}


def _write_text(path, text):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
        f.flush()
        os.fsync(f.fileno())


def fetch_month(out, month, base, opener, log):
    """Downloads and checks both databases of `month`, then puts them, the notice and the manifest in place."""
    try:
        out.mkdir(parents=True, exist_ok=True)
    except OSError as e:
        raise FetchError("%s: %s" % (out, e)) from None
    temp = {}
    try:
        files = {}
        for key, stem, final, kind in DATABASES:
            name = "%s-%s.mmdb.gz" % (stem, month)
            temp[final] = out / ("%s.%d.tmp" % (final, os.getpid()))
            size, sha1 = download(base + name, temp[final], opener)
            try:
                ty = check_file(temp[final], kind)
            except MmdbError as e:
                raise MmdbError("%s: %s" % (name, e)) from None
            log("%s: %d bytes unpacked, SHA-1 %s, %s" % (name, size, sha1, ty))
            files[key] = {"file": final, "bytes": size, "sha1": sha1}
        # The shape crates/geo/src/files.rs reads.
        manifest = {"month": month, "city": files["city"], "asn": files["asn"], "source": base}
        for final, text in ((NOTICE, notice(month, base)), (MANIFEST, json.dumps(manifest, indent=2) + "\n")):
            temp[final] = out / ("%s.%d.tmp" % (final, os.getpid()))
            _write_text(temp[final], text)
        # The manifest goes last: it names the month of the set.
        for final in ("dbip-city-lite.mmdb", "dbip-asn-lite.mmdb", NOTICE, MANIFEST):
            try:
                os.replace(temp[final], out / final)
            except OSError as e:
                raise FetchError("cannot put %s in place (%s): is a program using %s?" % (final, e, out)) from None
    except OSError as e:
        raise FetchError(str(e)) from None
    finally:
        for path in temp.values():
            try:
                path.unlink()
            except OSError:
                pass


def fetch(out, month=None, base=BASE, opener=urllib.request.urlopen, log=print):
    """Fetches the databases into `out`: of `month`, or of the current month (UTC) and, when DB-IP has not
    published that one yet, of the month before. Returns the month."""
    if month is None:
        now = current_month()
        tries = [now, prev_month(now)]
    else:
        tries = [month]
    for i, m in enumerate(tries):
        try:
            fetch_month(Path(out), m, base, opener, log)
            return m
        except NotPublished as e:
            if i + 1 == len(tries):
                if i:
                    raise NotPublished("DB-IP has published neither %s nor %s (%s)" % (tries[0], m, e)) from None
                raise
            log("%s: not published yet, trying %s" % (e, tries[i + 1]))


# ---------- self-test: tiny databases made in memory, a stand-in for the network ----------


class _U:
    """An unsigned integer of MMDB type `kind` (5 uint16, 6 uint32, 9 uint64, 10 uint128), for the test encoder."""

    def __init__(self, kind, n):
        self.kind, self.n = kind, n


class _P:
    """A pointer to an offset, for the test encoder."""

    def __init__(self, to):
        self.to = to


def _head(kind, size):
    if size < 29:
        first, extra = size, b""
    elif size < 285:
        first, extra = 29, bytes([size - 29])
    elif size < 65821:
        first, extra = 30, (size - 285).to_bytes(2, "big")
    else:
        first, extra = 31, (size - 65821).to_bytes(3, "big")
    if kind < 8:
        return bytes([kind << 5 | first]) + extra
    return bytes([first, kind - 7]) + extra


def _enc(v, out):
    """Appends `v` to `out` in the MMDB data format; returns where it starts."""
    at = len(out)
    if isinstance(v, _P):
        if v.to < 2048:
            out += bytes([0x20 | v.to >> 8, v.to & 0xFF])
        elif v.to < 526336:
            t = v.to - 2048
            out += bytes([0x28 | t >> 16]) + (t & 0xFFFF).to_bytes(2, "big")
        elif v.to < 526336 + (1 << 27):
            t = v.to - 526336
            out += bytes([0x30 | t >> 24]) + (t & 0xFFFFFF).to_bytes(3, "big")
        else:
            out += b"\x38" + v.to.to_bytes(4, "big")
    elif isinstance(v, bool):
        out += _head(14, int(v))
    elif isinstance(v, _U):
        raw = v.n.to_bytes((v.n.bit_length() + 7) // 8, "big")
        out += _head(v.kind, len(raw)) + raw
    elif isinstance(v, int):
        out += _head(8, 4) + v.to_bytes(4, "big", signed=True)
    elif isinstance(v, float):
        out += _head(3, 8) + struct.pack(">d", v)
    elif isinstance(v, str):
        raw = v.encode("utf-8")
        out += _head(2, len(raw)) + raw
    elif isinstance(v, bytes):
        out += _head(4, len(v)) + v
    elif isinstance(v, dict):
        out += _head(7, len(v))
        for key, item in v.items():
            _enc(key, out)
            _enc(item, out)
    elif isinstance(v, list):
        out += _head(11, len(v))
        for item in v:
            _enc(item, out)
    else:
        raise TypeError(v)
    return at


def _bytes(v):
    out = bytearray()
    _enc(v, out)
    return bytes(out)


def _tiny_mmdb(database_type, major=2, nodes=1):
    """A MaxMind DB file with an empty search tree of one node: its metadata is all this script reads. The
    description points back at the database type, as a writer that shares strings would do."""
    meta = bytearray()
    fields = [
        ("binary_format_major_version", _U(5, major)),
        ("binary_format_minor_version", _U(5, 0)),
        ("build_epoch", _U(9, 1790812800)),
        ("database_type", database_type),
        ("description", None),
        ("ip_version", _U(5, 6)),
        ("languages", ["en", "ru"]),
        ("node_count", _U(6, nodes)),
        ("record_size", _U(5, 24)),
    ]
    meta += _head(7, len(fields))
    type_at = None
    for key, v in fields:
        _enc(key, meta)
        if key == "description":
            v = {"en": _P(type_at)}
        at = _enc(v, meta)
        if key == "database_type":
            type_at = at
    # Both 24-bit records of the one node say "no data" (the node count); then 16 zero bytes and no data.
    return (1).to_bytes(3, "big") * 2 + bytes(16) + MARKER + bytes(meta)


def _check(ok, what):
    if not ok:
        raise AssertionError(what)


def _raises(error, fn, what):
    try:
        fn()
    except error as e:
        return str(e)
    raise AssertionError("%s: no %s" % (what, error.__name__))


def self_test():
    """Checks the months, the MMDB metadata reader and whole runs on tiny databases made in memory, with a stand-in
    for the network: no request leaves the computer."""
    # Months.
    for good in ("2026-10", "2026-01", "2026-12", "1999-09"):
        _check(is_month(good), good)
    for bad in ("2026-13", "2026-00", "2026-1", "26-10", "2026/10", "2026-1x", "", "\uff12\uff10\uff12\uff16-10"):
        _check(not is_month(bad), repr(bad))
    _check(prev_month("2026-10") == "2026-09", "prev 2026-10")
    _check(prev_month("2026-01") == "2025-12", "prev 2026-01")
    utc = datetime.timezone.utc
    _check(current_month(datetime.datetime(2026, 10, 31, 23, 59, 59, tzinfo=utc)) == "2026-10", "end of October")
    _check(current_month(datetime.datetime(2026, 11, 1, 0, 0, tzinfo=utc)) == "2026-11", "start of November")
    # 02:00 on 1 November in Baku (UTC+4) is still October in UTC.
    baku = datetime.timezone(datetime.timedelta(hours=4))
    _check(current_month(datetime.datetime(2026, 11, 1, 2, 0, tzinfo=baku)) == "2026-10", "UTC, not local time")
    _check(is_month(current_month()), "now")

    # Metadata laid out by hand from the format description, not by the test encoder: a map of three pairs, a
    # 27-byte key ("["), uint16 2, an 11-byte key ("K"), uint64 (extended type 2) of 4 bytes, a 13-byte key ("M"),
    # a 14-byte string ("N"). A marker earlier in the file is data: the last one counts.
    literal = (
        b"\xe3"
        b"[binary_format_major_version\xa1\x02"
        b"Kbuild_epoch\x04\x02\x6a\xb9\x1e\x80"
        b"Mdatabase_type\x4eDBIP-City-Lite"
    )
    meta = read_metadata(b"\0" * 9 + MARKER + b"\x41x" + MARKER + literal)
    _check(
        meta == {"binary_format_major_version": 2, "build_epoch": 0x6AB91E80, "database_type": "DBIP-City-Lite"},
        "hand-made metadata: %r" % meta,
    )

    # Every type and size form the reader knows, through the test encoder.
    sample = {
        "short": "x" * 28, "29": "y" * 29, "284": "z" * 284, "285": "w" * 285, "65820": "v" * 65820,
        "65821": "u" * 65821, "long": "t" * 70000, "utf8": "Баку", "double": 40.4093, "bytes": b"\x00\xff",
        "u16": _U(5, 65535), "u32": _U(6, 4294967295), "u64": _U(9, 1 << 63), "u128": _U(10, (1 << 128) - 1),
        "zero": _U(6, 0), "i32": -5, "true": True, "false": False, "list": [_U(5, 1), "two", [], {}],
        "nested": {"names": {"en": "Baku", "ru": "Баку"}},
    }
    want = dict(sample)
    for key in ("u16", "u32", "u64", "u128", "zero"):
        want[key] = sample[key].n
    want["list"] = [1, "two", [], {}]
    buf = _bytes(sample)
    got, end = decode(buf)
    _check(got == want and end == len(buf), "round trip of every type")
    # A float (extended type 8) and int32 (extended type 1), by hand.
    _check(decode(b"\x04\x08" + struct.pack(">f", 1.5))[0] == 1.5, "float")
    _check(decode(b"\x04\x01\xff\xff\xff\xfb")[0] == -5, "int32")
    # Shorter than 4 bytes, an int32 is padded with zeros: 0xff is 255.
    _check(decode(b"\x01\x01\xff")[0] == 255, "short int32")
    # Pointers of all four sizes: to 0 and 3000 through the encoder, to 530000 by hand (sizes 2 and 3).
    strings = bytearray()
    _enc("near", strings)
    strings += bytes(3000 - len(strings))
    _enc("further", strings)
    strings += bytes(530000 - len(strings))
    _enc("far away", strings)
    strings = bytes(strings)
    _check(_bytes(_P(530000)) == b"\x30\x00\x0e\x50", "pointer encoding")
    for p, text in (
        (_bytes(_P(0)), "near"),
        (_bytes(_P(3000)), "further"),
        (b"\x30\x00\x0e\x50", "far away"),
        (b"\x38\x00\x08\x16\x50", "far away"),
    ):
        value, end = decode(strings + p, len(strings))
        _check(value == text and end == len(strings) + len(p), "pointer %r" % p)
    for broken in (
        b"",  # nothing
        b"\x5b",  # a 27-byte string without its bytes
        b"\xe1\x41",  # a map whose key is cut short
        b"\xe1\xa1\x01\x41x",  # a map key that is a number
        b"\x04\x02\x01",  # a 4-byte uint64 with one byte
        b"\x68\x00",  # a double cut short
        b"\x41\xff",  # a string that is not UTF-8
        b"\xa3\x01\x02\x03",  # a 3-byte uint16
        b"\x00\x06",  # the end marker type
        b"\x20\x40",  # a pointer past the end
        b"\x20\x00",  # a pointer to itself
    ):
        _raises(MmdbError, lambda b=broken: decode(b), "broken %r" % broken)

    # Whole files: the type check, as the program does it, and the sanity checks.
    city, asn = _tiny_mmdb("DBIP-City-Lite"), _tiny_mmdb("DBIP-ASN-Lite (compat=GeoLite2-ASN)")
    meta = read_metadata(city)
    _check(meta["description"] == {"en": "DBIP-City-Lite"} and meta["languages"] == ["en", "ru"], "pointer in metadata")
    _check(meta["build_epoch"] == 1790812800 and meta["node_count"] == 1, "metadata numbers")
    _check(check_metadata(meta, "City", len(city)) == "DBIP-City-Lite", "city")
    _check(check_metadata(read_metadata(asn), "ASN", len(asn)).startswith("DBIP-ASN-Lite"), "asn")
    _check(check_metadata(read_metadata(_tiny_mmdb("dbip-city-lite")), "City", 100) == "dbip-city-lite", "any case")
    _raises(MmdbError, lambda: check_metadata(read_metadata(asn), "City", len(asn)), "an ASN file as City")
    _raises(MmdbError, lambda: check_metadata(meta, "ASN", len(city)), "a City file as ASN")
    three = _tiny_mmdb("DBIP-City-Lite", major=3)
    _raises(MmdbError, lambda: check_metadata(read_metadata(three), "City", len(three)), "format 3")
    big = _tiny_mmdb("DBIP-City-Lite", nodes=1000)
    _raises(MmdbError, lambda: check_metadata(read_metadata(big), "City", len(big)), "a tree that does not fit")
    _raises(MmdbError, lambda: read_metadata(city.replace(MARKER, b"\xab\xcd\xefMaxMind.org")), "no marker")
    _raises(MmdbError, lambda: read_metadata(city[:-3]), "metadata cut short")
    stub = _Limited(io.BytesIO(b"x" * 10), 5, "stub")
    _raises(FetchError, lambda: stub.read(100), "over the size limit")

    # Whole runs through a stand-in for the network.
    now = current_month()
    before = prev_month(now)
    gz = gzip.compress

    def url(stem, month):
        return BASE + "%s-%s.mmdb.gz" % (stem, month)

    with tempfile.TemporaryDirectory(prefix="fetch-geo-test-") as tmp:
        out = Path(tmp) / "geo"
        out.mkdir()
        # A set from an earlier run, which stays as it is until a new one is complete.
        (out / "dbip-city-lite.mmdb").write_bytes(b"old city")
        (out / MANIFEST).write_bytes(b'{"month":"2000-01"}')

        def files():
            return {n: (out / n).read_bytes() for n in sorted(os.listdir(out))}

        def run(served, month=None):
            """fetch() with `served` as the server (URL to body or status); returns the month, or the error after
            checking that the folder is as it was, and the URLs asked for."""
            seen, start = [], files()

            def opener(request, timeout=None):
                seen.append(request.full_url)
                _check(request.get_header("User-agent") == USER_AGENT and timeout == 60, "request")
                # Nothing of this run is in place before both databases are checked.
                now_there = {n: b for n, b in files().items() if not n.endswith(".tmp")}
                _check(now_there == start, "written in place: %r" % sorted(now_there))
                body = served.get(request.full_url, 404)
                if isinstance(body, int):
                    raise urllib.error.HTTPError(request.full_url, body, "status %d" % body, None, None)
                return io.BytesIO(body)

            try:
                return fetch(out, month, opener=opener, log=lambda line: None), seen
            except FetchError as e:
                _check(files() == start, "a failed run left %r" % sorted(files()))
                return e, seen

        # A month asked for by name that is not out: no other month is tried.
        e, seen = run({}, now)
        _check(isinstance(e, NotPublished) and seen == [url("dbip-asn-lite", now)], "named month: %r" % seen)
        # Neither the current month nor the one before.
        e, seen = run({})
        _check(isinstance(e, NotPublished) and now in str(e) and before in str(e), str(e))
        _check(seen == [url("dbip-asn-lite", now), url("dbip-asn-lite", before)], "neither: %r" % seen)
        # A server error does not say the month is not out: no fallback.
        e, seen = run({url("dbip-asn-lite", now): 503})
        _check(type(e) is FetchError and "HTTP 503" in str(e) and len(seen) == 1, str(e))
        # A file of the wrong type, a broken gzip, a gzip cut short: nothing is put in place.
        for served, what in (
            ({url("dbip-asn-lite", now): gz(asn), url("dbip-city-lite", now): gz(asn)}, "expected City"),
            ({url("dbip-asn-lite", now): gz(asn), url("dbip-city-lite", now): b"not gzip"}, "dbip-city-lite"),
            ({url("dbip-asn-lite", now): gz(asn)[:-20]}, "dbip-asn-lite"),
        ):
            e, seen = run(served)
            _check(isinstance(e, FetchError) and what in str(e), "%s: %s" % (what, e))

        # The current month is not out yet: the month before.
        month, seen = run({url("dbip-city-lite", before): gz(city), url("dbip-asn-lite", before): gz(asn)})
        _check(month == before, "fallback: %s" % month)
        _check(seen == [url("dbip-asn-lite", now), url("dbip-asn-lite", before), url("dbip-city-lite", before)],
               "requests %r" % seen)
        names = sorted(["dbip-asn-lite.mmdb", "dbip-city-lite.mmdb", MANIFEST, NOTICE])
        _check(sorted(os.listdir(out)) == names, "files %r" % os.listdir(out))
        _check((out / "dbip-city-lite.mmdb").read_bytes() == city, "city in place")
        _check((out / "dbip-asn-lite.mmdb").read_bytes() == asn, "asn in place")
        text = (out / MANIFEST).read_text(encoding="utf-8")
        manifest = json.loads(text)
        _check(list(manifest) == ["month", "city", "asn", "source"], "manifest keys %r" % list(manifest))
        _check(manifest == {
            "month": before,
            "city": {"file": "dbip-city-lite.mmdb", "bytes": len(city), "sha1": hashlib.sha1(city).hexdigest()},
            "asn": {"file": "dbip-asn-lite.mmdb", "bytes": len(asn), "sha1": hashlib.sha1(asn).hexdigest()},
            "source": BASE,
        }, "manifest %s" % text)
        text = (out / NOTICE).read_text(encoding="utf-8")
        for part in (
            "IP Geolocation by DB-IP: https://db-ip.com",
            "(CC BY 4.0): https://creativecommons.org/licenses/by/4.0/",
            "DB-IP IP to City Lite, %s" % before,
            "dbip-city-lite-%s.mmdb.gz" % before,
            "not under the MIT or Apache-2.0 license",
            "not modified apart from decompression",
        ):
            _check(part in text, "notice: %s" % part)
        _check("\u2014" not in text, "notice: em dash")

        # A later run replaces the whole set by renaming; here of a month asked for by name.
        city2 = _tiny_mmdb("DBIP-City-Lite (newer)")
        month, seen = run({url("dbip-city-lite", now): gz(city2), url("dbip-asn-lite", now): gz(asn)}, now)
        _check(month == now and len(seen) == 2, "named month: %s" % month)
        _check((out / "dbip-city-lite.mmdb").read_bytes() == city2, "city replaced")
        _check(json.loads((out / MANIFEST).read_text(encoding="utf-8"))["month"] == now, "manifest replaced")
        _check(sorted(os.listdir(out)) == names, "files after the second run %r" % os.listdir(out))


def main(argv=None):
    parser = argparse.ArgumentParser(description="Download DB-IP City Lite and ASN Lite for the route map.")
    parser.add_argument("--month", help="YYYY-MM; default: the current month (UTC), else the month before")
    parser.add_argument("--out", default=".target/geo", help="output folder (default: .target/geo)")
    parser.add_argument("--self-test", action="store_true", help="check the reader and the months, offline")
    args = parser.parse_args(argv)
    if args.self_test:
        self_test()
        print("self-test passed")
        return
    if args.month is not None and not is_month(args.month):
        parser.error("--month: not a month: %r" % args.month)
    try:
        month = fetch(Path(args.out), args.month)
    except FetchError as e:
        sys.exit("fetch_geo: %s" % e)
    print("DB-IP Lite %s in %s" % (month, args.out))


if __name__ == "__main__":
    main()

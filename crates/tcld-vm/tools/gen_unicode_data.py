#!/usr/bin/env python3
"""Generate crates/tcld-vm/src/unicode_data.rs from Tcl's tclUniData.c.

Tcl's case conversions and `string is` character classes are driven by the
tables that Tcl's tools/uniParse.tcl generates into generic/tclUniData.c. This
script transcribes those tables verbatim, together with the constants the
lookup macros use, so that `tcld_vm::unicode` can mirror Tcl's lookups exactly.

It refuses to run if the table encoding or the lookup macros differ from the
ones `tcld_vm::unicode` mirrors, and it checks that every in-range case mapping
is a Unicode scalar value (so the Rust side can return `char`).

Usage (standard library only; works from any directory):

    python3 crates/tcld-vm/tools/gen_unicode_data.py           # rewrite
    python3 crates/tcld-vm/tools/gen_unicode_data.py --check   # verify
"""

import argparse
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[3]
DEFAULT_TCL_DIR = ROOT / "third_party" / "tcl" / "src" / "tcl-core-9-0-2"
DEFAULT_OUTPUT = ROOT / "crates" / "tcld-vm" / "src" / "unicode_data.rs"

# The table sections guarded by this condition hold the code points past
# U+FFFF. The condition is true whenever TCL_MAJOR_VERSION > 8.
TCL9_CONDITION = "#if TCL_UTF_MAX > 3 || TCL_MAJOR_VERSION > 8 || TCL_MINOR_VERSION > 6"

# Macros whose exact meaning src/unicode.rs mirrors.
REQUIRED_LINES = (
    "#define GetCaseType(info) (((info) & 0xE0) >> 5)",
    "#define GetCategory(ch) (GetUniCharInfo(ch) & 0x1F)",
    "#define GetDelta(info) ((info) >> 8)",
    "#   define GetUniCharInfo(ch) (groups[groupMap[pageMap[((ch) & 0x1FFFFF)"
    " >> OFFSET_BITS] | ((ch) & ((1 << OFFSET_BITS)-1))]])",
)

# Category names src/unicode.rs refers to; the numbering comes from the file.
REQUIRED_CATEGORIES = (
    "UPPERCASE_LETTER", "LOWERCASE_LETTER", "TITLECASE_LETTER",
    "MODIFIER_LETTER", "OTHER_LETTER", "NON_SPACING_MARK", "ENCLOSING_MARK",
    "COMBINING_SPACING_MARK", "DECIMAL_DIGIT_NUMBER", "LETTER_NUMBER",
    "OTHER_NUMBER", "SPACE_SEPARATOR", "LINE_SEPARATOR",
    "PARAGRAPH_SEPARATOR", "CONTROL", "FORMAT", "CONNECTOR_PUNCTUATION",
    "DASH_PUNCTUATION", "OPEN_PUNCTUATION", "CLOSE_PUNCTUATION",
    "INITIAL_QUOTE_PUNCTUATION", "FINAL_QUOTE_PUNCTUATION",
    "OTHER_PUNCTUATION", "MATH_SYMBOL", "CURRENCY_SYMBOL", "MODIFIER_SYMBOL",
    "OTHER_SYMBOL",
)


class GenerationError(Exception):
    pass


def parse_array(source, c_type, name):
    pattern = r"static const %s %s\[\] = \{\n(.*?)\n\};" % (re.escape(c_type), name)
    match = re.search(pattern, source, re.S)
    if not match:
        raise GenerationError(f"array {name} ({c_type}) not found")
    values = []
    for line in match.group(1).split("\n"):
        text = line.strip()
        if text.startswith("#"):
            if text == TCL9_CONDITION or text.startswith("#endif"):
                continue
            raise GenerationError(f"unexpected preprocessor line in {name}: {text}")
        for token in text.split(","):
            token = token.strip()
            if not token:
                continue
            if not re.fullmatch(r"-?[0-9]+", token):
                raise GenerationError(f"unexpected token in {name}: {token!r}")
            values.append(int(token, 10))
    return values


def parse_categories(source):
    match = re.search(r"\nenum \{\n(.*?)\n\};", source, re.S)
    if not match:
        raise GenerationError("category enum not found")
    names = [name.strip() for name in match.group(1).split(",")]
    if not all(re.fullmatch(r"[A-Z_]+", name) for name in names):
        raise GenerationError(f"unexpected category enum: {names}")
    missing = [name for name in REQUIRED_CATEGORIES if name not in names]
    if missing:
        raise GenerationError(f"category enum lacks {missing}")
    if len(names) > 32:
        raise GenerationError("categories no longer fit the 5-bit category field")
    return names


def parse_define(source, name, value_pattern, what):
    match = re.search(r"#\s*define\s+%s\s+%s" % (name, value_pattern), source)
    if not match:
        raise GenerationError(f"{what} not found")
    return match.group(1)


def parse_tables(tcl_dir):
    generic = tcl_dir / "generic"
    source = (generic / "tclUniData.c").read_text(encoding="utf-8")
    header = (generic / "tcl.h").read_text(encoding="utf-8")

    major = int(parse_define(header, "TCL_MAJOR_VERSION", r"([0-9]+)", "TCL_MAJOR_VERSION"))
    if major <= 8:
        raise GenerationError("only Tcl 9 tables are supported (TCL_UTF_MAX sections)")
    patch_level = parse_define(header, "TCL_PATCH_LEVEL", r'"([^"]+)"', "TCL_PATCH_LEVEL")

    for line in REQUIRED_LINES:
        if line not in source:
            raise GenerationError(f"lookup macro changed; expected: {line}")
    limit_match = re.search(
        re.escape(TCL9_CONDITION)
        + r"\n#\s*define UNICODE_OUT_OF_RANGE\(ch\) \(\(\(ch\) & 0x1FFFFF\) >= (0x[0-9A-Fa-f]+)\)\n",
        source,
    )
    if not limit_match:
        raise GenerationError("UNICODE_OUT_OF_RANGE (Tcl 9 branch) not found")

    return {
        "patch_level": patch_level,
        "offset_bits": int(parse_define(source, "OFFSET_BITS", r"([0-9]+)", "OFFSET_BITS")),
        "limit": int(limit_match.group(1), 16),
        "page_map": parse_array(source, "unsigned short", "pageMap"),
        "group_map": parse_array(source, "unsigned char", "groupMap"),
        "groups": parse_array(source, "int", "groups"),
        "categories": parse_categories(source),
    }


def validate(tables):
    shift = tables["offset_bits"]
    page_size = 1 << shift
    limit = tables["limit"]
    page_map = tables["page_map"]
    group_map = tables["group_map"]
    groups = tables["groups"]

    if limit % page_size or not 0x10000 <= limit <= 0x110000:
        raise GenerationError(f"unexpected UNICODE_OUT_OF_RANGE limit {limit:#x}")
    if len(page_map) != limit >> shift:
        raise GenerationError(
            f"pageMap has {len(page_map)} entries; limit {limit:#x} needs {limit >> shift}")
    for page in page_map:
        if page % page_size or page + page_size > len(group_map) or page > 0xFFFF:
            raise GenerationError(f"pageMap entry {page} does not address a groupMap page")
    if any(not 0 <= group < len(groups) or group > 0xFF for group in group_map):
        raise GenerationError("groupMap entry out of range")
    if any(not -(1 << 31) <= info < (1 << 31) for info in groups):
        raise GenerationError("groups entry does not fit an int")

    # Every in-range mapping must be a Unicode scalar value. This mirrors
    # Tcl_UniCharToUpper/ToLower/ToTitle in tclUtf.c.
    def is_scalar(value):
        return 0 <= value <= 0x10FFFF and not 0xD800 <= value <= 0xDFFF

    for ch in range(limit):
        if 0xD800 <= ch <= 0xDFFF:
            continue
        info = groups[group_map[page_map[ch >> shift] | (ch & (page_size - 1))]]
        mode = (info & 0xE0) >> 5
        delta = info >> 8
        upper = ch - delta if mode & 0x4 else ch
        lower = ch + delta if (mode & 0x2) and mode != 0x7 else ch
        if mode & 0x1:
            title = ch + (-1 if mode & 0x4 else 1) if mode != 0x7 else ch
        elif mode == 0x4:
            title = ch - delta
        else:
            title = ch
        for kind, value in (("upper", upper), ("lower", lower), ("title", title)):
            if not is_scalar(value & 0x1FFFFF):
                raise GenerationError(f"U+{ch:04X} {kind} maps to {value:#x}")


def rust_array(name, rust_type, values, per_line, docs):
    lines = [f"/// {doc}" if doc else "///" for doc in docs]
    lines.append("#[rustfmt::skip]")
    lines.append(f"pub(crate) static {name}: [{rust_type}; {len(values)}] = [")
    for start in range(0, len(values), per_line):
        chunk = values[start:start + per_line]
        lines.append("    " + ", ".join(str(value) for value in chunk) + ",")
    lines.append("];")
    return "\n".join(lines)


def render(tables, source_label):
    version = tables["patch_level"]
    shift = tables["offset_bits"]
    limit = tables["limit"]
    category_lines = "\n".join(
        f"    pub(crate) const {name}: u32 = {index};"
        for index, name in enumerate(tables["categories"])
    )
    sections = [
        f"""\
// @generated by crates/tcld-vm/tools/gen_unicode_data.py from Tcl {version}'s
// {source_label}
// (itself generated by Tcl's tools/uniParse.tcl). Do not edit by hand.
//
// Regenerate from the repository root with
//     python3 crates/tcld-vm/tools/gen_unicode_data.py
// and check that this file is current with `--check`.

//! Tcl {version} Unicode character tables, transcribed verbatim from
//! `generic/tclUniData.c`; [`crate::unicode`] holds the lookup logic.
//!
//! The information about a code point `ch` below [`UNICODE_LIMIT`] is
//! `GROUPS[GROUP_MAP[PAGE_MAP[ch >> OFFSET_BITS] | (ch & (2^OFFSET_BITS - 1))]]`
//! (Tcl's `GetUniCharInfo`). Bits 0-4 of that value hold the category, bits
//! 5-7 the case delta type, and bits 8-31 the signed case delta.

#![forbid(unsafe_code)]

/// `TCL_PATCH_LEVEL` of the Tcl release these tables were taken from.
pub const TCL_PATCH_LEVEL: &str = "{version}";

/// `OFFSET_BITS`: the low bits of a code point that index into a page.
pub(crate) const OFFSET_BITS: u32 = {shift};

/// Code points at or above this value are `UNICODE_OUT_OF_RANGE` for Tcl.
pub(crate) const UNICODE_LIMIT: u32 = 0x{limit:X};

/// Tcl's character category numbers (the anonymous `enum` in `tclUniData.c`).
#[allow(dead_code)] // The lookup logic does not name every category.
pub(crate) mod category {{
{category_lines}
}}""",
        rust_array(
            "PAGE_MAP", "u16", tables["page_map"], 12,
            ["`pageMap`: page number (`ch >> OFFSET_BITS`) to the offset of its",
             "unique page in [`GROUP_MAP`]."]),
        rust_array(
            "GROUP_MAP", "u8", tables["group_map"], 16,
            ["`groupMap`: unique pages of group numbers (indices into [`GROUPS`]),",
             f"{1 << shift} entries per page."]),
        rust_array(
            "GROUPS", "i32", tables["groups"], 8,
            ["`groups`: the packed category, case delta type and case delta."]),
    ]
    return "\n\n".join(sections) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--tcl-dir", type=pathlib.Path, default=DEFAULT_TCL_DIR,
                        help="Tcl source tree (default: %(default)s)")
    parser.add_argument("--output", type=pathlib.Path, default=DEFAULT_OUTPUT,
                        help="generated Rust file (default: %(default)s)")
    parser.add_argument("--check", action="store_true",
                        help="fail instead of writing if the output is stale")
    args = parser.parse_args()

    tcl_dir = args.tcl_dir.resolve()
    try:
        tables = parse_tables(tcl_dir)
        validate(tables)
    except (GenerationError, OSError) as error:
        sys.exit(f"gen_unicode_data: {error}")

    source = tcl_dir / "generic" / "tclUniData.c"
    try:
        source_label = source.relative_to(ROOT).as_posix()
    except ValueError:
        source_label = f"generic/{source.name}"
    text = render(tables, source_label)

    output = args.output.resolve()
    if args.check:
        current = output.read_text(encoding="utf-8") if output.exists() else None
        if current != text:
            sys.exit(f"gen_unicode_data: {output} is stale; rerun without --check")
        print(f"{output} is current (Tcl {tables['patch_level']})")
        return
    output.write_text(text, encoding="utf-8")
    print(f"wrote {output} (Tcl {tables['patch_level']}: {len(tables['page_map'])} pages, "
          f"{len(tables['group_map'])} group map entries, {len(tables['groups'])} groups)")


if __name__ == "__main__":
    main()

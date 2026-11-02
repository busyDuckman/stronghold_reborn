#!/usr/bin/env python3
# manualy detect code smells by examining all .rs files (excluding /bin/* files and vendored files.)
#
# enum for every smell, easily extensable
# outputs a list as per:
#
#   SMELL: "".unwrap() used in production code
#     - foo/bar/main.rs#37        result.unwrap()
#     - foo/bar/main.rs#39        name.unwrap()
#
# Smells:
#  1) use of .unwrap()
# only permisable in support code and in unit tests

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from pathlib import Path

# Folders we never scan (relative to repo root).
EXCLUDED_DIRS = {
    "target",
    "vendor",
    ".venv",
    ".venv-1",
    ".git",
    "node_modules",
}

# Path globs that are considered "support code", .unwrap() etc. is fine here.
SUPPORT_PATH_FRAGMENTS = (
    "/src/bin/",
    "/tests/",
    "/benches/",
    "/examples/",
    "/scratchpad/",
    "/utils/",          # standalone dev tools: printing to stdout is their whole job
)

# Per-line opt-out marker. A line containing this string is never flagged.
IGNORE_TOKEN = "IGNORE_SMELL"

# Per-file token registration. A comment like
#     // IGNORE_SMELL: token="pallate.get"  this is ok here
# registers the substring `pallate.get` as an opt-out token for the rest of
# the file. Any later line containing that substring is exempt from all smells.
_TOKEN_DECL_RE = re.compile(r'IGNORE_SMELL\s*:\s*token\s*=\s*"([^"]+)"')


@dataclass
class Hit:
    path: Path
    line_no: int
    text: str


@dataclass
class FileContext:
    """Pre-computed per-file context shared by all smell checks."""
    path: Path
    lines: list[str]
    # Line numbers (1-based) that fall inside `#[cfg(test)] mod ...` blocks
    # or inside `#[test]` / `#[tokio::test]` functions.
    test_line_set: set[int] = field(default_factory=set)
    # Tokens registered via `IGNORE_SMELL: token="..."` comments, mapped to the
    # 1-based line number where they were declared. A line at or after that
    # line containing the substring is exempt from all smells.
    ignore_tokens: dict[str, int] = field(default_factory=dict)
    # 1-based start line -> run length for comment runs flagged as AI bloat
    # (>= 3 consecutive whole-line comments with a backtick somewhere in them).
    comment_bloat_starts: dict[int, int] = field(default_factory=dict)

    @classmethod
    def load(cls, path: Path) -> FileContext:
        text = path.read_text(encoding="utf-8", errors="replace")
        lines = text.splitlines()
        ctx = cls(path=path, lines=lines)
        ctx.test_line_set = _compute_test_line_set(lines)
        ctx.ignore_tokens = _compute_ignore_tokens(lines)
        ctx.comment_bloat_starts = _compute_comment_bloat_blocks(lines)
        return ctx

    def is_in_test(self, line_no: int) -> bool:
        return line_no in self.test_line_set

    def is_support_file(self) -> bool:
        p = "/" + str(self.path).replace("\\", "/").lstrip("/")
        return any(frag in p for frag in SUPPORT_PATH_FRAGMENTS)

    def is_line_ignored(self, line_no: int, line: str) -> bool:
        """True if the line opts out of smell checks via IGNORE_SMELL or via
        a previously-registered per-file token."""
        if IGNORE_TOKEN in line:
            return True
        for tok, decl_line in self.ignore_tokens.items():
            if line_no >= decl_line and tok in line:
                return True
        return False


def _compute_ignore_tokens(lines: list[str]) -> dict[str, int]:
    """Scan the file for `IGNORE_SMELL: token="..."` directives. Returns a
    mapping from token substring to the 1-based line number it was declared on."""
    tokens: dict[str, int] = {}
    for idx, line in enumerate(lines, start=1):
        for m in _TOKEN_DECL_RE.finditer(line):
            tok = m.group(1)
            # First declaration wins, later redeclarations don't shrink scope.
            if tok and tok not in tokens:
                tokens[tok] = idx
    return tokens


# A backtick that counts toward comment bloat. Backticks used as an apostrophe
# (can`t, it`s, Bob`s, dogs`) sit against a word letter, so a letter immediately
# before the backtick disqualifies it. A markdown code backtick opens against a
# space or punctuation, so it still matches.
_CODE_BACKTICK_RE = re.compile(r"(?<![A-Za-z])`")
# Triple-backtick (or longer) code fences are legit usage examples, not inline
# AI prose, so we strip them out before counting code backticks.
_FENCE_RE = re.compile(r"`{3,}")


def _has_code_backtick(text: str) -> bool:
    return bool(_CODE_BACKTICK_RE.search(_FENCE_RE.sub("", text)))


def _compute_comment_bloat_blocks(lines: list[str]) -> dict[int, int]:
    """Find runs of >= 3 consecutive whole-line comments that contain at least
    one markdown-style backtick somewhere in the run. Apostrophe-backticks like
    can`t or Bobs` don't count, and neither do triple-backtick code fences
    (```), which are legit usage examples. That combination is the tell-tale
    sign of AI-padded prose (markdown backticks leaking into Rust comments).
    Returns a mapping from the run's 1-based start line to its length.

    Handles `//`-style line comments and `/* ... */` block comments. A trailing
    comment after code (`let x = 5; // foo`) is not a whole-line comment, so it
    never starts or extends a run."""
    blocks: dict[int, int] = {}
    run_start: int | None = None  # 1-based start of the current comment run
    run_has_backtick = False
    in_block_comment = False

    def flush(end_line: int) -> None:
        nonlocal run_start, run_has_backtick
        if run_start is not None and run_has_backtick:
            length = end_line - run_start + 1
            if length >= 3:
                blocks[run_start] = length
        run_start = None
        run_has_backtick = False

    for idx, raw in enumerate(lines, start=1):
        stripped = raw.strip()
        is_comment = False
        if in_block_comment:
            is_comment = True
            if "*/" in raw:
                in_block_comment = False
        elif stripped.startswith("//"):
            is_comment = True
        elif stripped.startswith("/*"):
            is_comment = True
            if "*/" not in stripped:
                in_block_comment = True

        if is_comment:
            if run_start is None:
                run_start = idx
            if _has_code_backtick(raw):
                run_has_backtick = True
        else:
            flush(idx - 1)

    flush(len(lines))
    return blocks


def _compute_test_line_set(lines: list[str]) -> set[int]:
    """Return the set of 1-based line numbers that live inside test code.

    Catches two patterns:
      * `#[cfg(test)] mod foo { ... }` (entire block, including nested braces).
      * A function preceded by `#[test]`, `#[tokio::test]`, etc.
    """
    in_test_lines: set[int] = set()

    # Track brace depth so we can find the end of a `mod tests { ... }` block
    # without a full parser. Strings/comments containing braces will throw this
    # off, but it's good enough for the lint use-case.
    cfg_test_re = re.compile(r"#\[\s*cfg\(\s*test\s*\)\s*\]")
    test_attr_re = re.compile(r"#\[\s*(?:tokio::|async_std::)?test\b")
    mod_open_re = re.compile(r"\bmod\s+\w+\s*\{")
    fn_open_re = re.compile(r"\bfn\s+\w+[^\{;]*\{")

    i = 0
    n = len(lines)
    while i < n:
        line = lines[i]

        # `#[cfg(test)]` find the next `mod ... {` (possibly on same line)
        # and mark every line until the matching closing brace.
        if cfg_test_re.search(line):
            j = i
            # Walk forward to the opening brace of the mod.
            while j < n and not mod_open_re.search(lines[j]):
                j += 1
            if j < n:
                start, end = _scan_braces(lines, j)
                for k in range(start, end + 1):
                    in_test_lines.add(k + 1)  # store 1-based
                i = end + 1
                continue

        # `#[test]` etc. same idea but for a single fn.
        if test_attr_re.search(line):
            j = i
            while j < n and not fn_open_re.search(lines[j]):
                j += 1
            if j < n:
                start, end = _scan_braces(lines, j)
                for k in range(start, end + 1):
                    in_test_lines.add(k + 1)
                i = end + 1
                continue

        i += 1

    return in_test_lines


def _scan_braces(lines: list[str], start_idx: int) -> tuple[int, int]:
    """Given a line index containing the opening `{` of a block, return
    (start_idx, end_idx) covering the block. End is inclusive."""
    depth = 0
    seen_open = False
    for k in range(start_idx, len(lines)):
        for ch in lines[k]:
            if ch == "{":
                depth += 1
                seen_open = True
            elif ch == "}":
                depth -= 1
                if seen_open and depth == 0:
                    return start_idx, k
    return start_idx, len(lines) - 1


# --- Smells -----------------------------------------------------------------

class Smell(ABC):
    """Base class for a single code smell. Add a new smell by subclassing
    this and appending an instance to SMELLS at the bottom of the file."""

    title: str  # human-readable, shown in the report header

    @abstractmethod
    def check_line(self, ctx: FileContext, line_no: int, line: str) -> str | None:
        """Return a short excerpt to report, or None if the line is clean."""
        ...

    def applies_to_file(self, ctx: FileContext) -> bool:
        """Override to skip whole files (e.g. support code)."""
        return not ctx.is_support_file()


class UnwrapSmell(Smell):
    title = '.unwrap() used in production code'

    # `.unwrap()` but not `.unwrap_or`, `.unwrap_or_else`, `.unwrap_or_default`,
    # and not `.unwrap_err()` (which is conventional in tests/assertions but
    # we still flag it in production, it's the same hazard).
    _re = re.compile(r"\.unwrap\s*\(\s*\)")

    def check_line(self, ctx: FileContext, line_no: int, line: str) -> str | None:
        stripped = line.lstrip()
        if stripped.startswith("//"):
            return None
        if ctx.is_in_test(line_no):
            return None
        m = self._re.search(line)
        if not m:
            return None
        return stripped


class PrintSmell(Smell):
    title = 'Printing directly to stdout/stderr (incl. dbg!)'

    # `print!`, `println!`, `eprint!`, `eprintln!`, `dbg!`
    _re = re.compile(r"(^|[^A-Za-z0-9_])(e?print(ln)?|dbg)\s*!")

    def check_line(self, ctx: FileContext, line_no: int, line: str) -> str | None:
        stripped = line.lstrip()
        if stripped.startswith("//"):
            return None
        if ctx.is_in_test(line_no):
            return None
        m = self._re.search(line)
        if not m:
            return None
        return stripped


class NilUuidSmell(Smell):
    """Banned: `Uuid::nil()` (the all-zeros UUID) and `.is_nil()` tests against
    it. It is a silent sentinel that leaks through equality checks and DB lookups
    as if it were a real id. Testing `.is_nil()` is the same anti-pattern from
    the other side. Use a real `Option<Uuid>` / explicit sentinel type instead."""
    title = 'Uuid::nil() / .is_nil(), use Option<Uuid> or an explicit sentinel'

    _re = re.compile(r"((^|[^A-Za-z0-9_])Uuid::nil|\.is_nil)\s*\(\s*\)")

    def check_line(self, ctx: FileContext, line_no: int, line: str) -> str | None:
        stripped = line.lstrip()
        if stripped.startswith("//"):
            return None
        if ctx.is_in_test(line_no):
            return None
        if not self._re.search(line):
            return None
        return stripped


# ---------------------------------------------------------------------------
# PROJECT SMELLS go below. One class per banned idiom, appended to SMELLS.
# Keep them narrow and greppable; a smell that fires on legitimate code trains
# people to ignore the whole report.
#
# Worked example (from tick_words, where the global clock is the only time source):
#
#   class GetFrameTimeSmell(Smell):
#       """Banned: macroquad's `get_frame_time()`. Use `frame_now()` instead."""
#       title = 'get_frame_time(), use frame_now() / GLOBAL_CLOCK instead'
#       _re = re.compile(r"(^|[^A-Za-z0-9_])get_frame_time\s*\(")
#
#       def check_line(self, ctx, line_no, line):
#           stripped = line.lstrip()
#           if stripped.startswith("//") or not self._re.search(line):
#               return None
#           return stripped
#
#       def applies_to_file(self, ctx) -> bool:
#           return True     # banned everywhere, including bin/ and tests
# ---------------------------------------------------------------------------


class CommentBloatSmell(Smell):
    """AI comment bloat: a run of >= 3 whole-line comments with a backtick in
    it. Backticks in comments are against project style, and a multi-line block
    of them is the signature of machine-padded prose. Reported once per run, on
    its first line, put IGNORE_SMELL there to silence a deliberate one.

    The run is detected in FileContext.comment_bloat_starts; here we just look
    up whether this line begins a flagged run."""
    title = 'AI comment bloat (>= 3 comment lines containing a backtick)'

    def check_line(self, ctx: FileContext, line_no: int, line: str) -> str | None:
        length = ctx.comment_bloat_starts.get(line_no)
        if length is None:
            return None
        return f"{line.strip()}   [{length}-line comment]"


SMELLS: list[Smell] = [
    UnwrapSmell(),
    PrintSmell(),
    NilUuidSmell(),
    CommentBloatSmell(),
    # project smells go here
]


# --- Scanner ---------------------------------------------------------------

def _git_pending_files(root: Path) -> set[Path]:
    """Absolute, resolved paths of every file with pending git changes: staged,
    unstaged, and new untracked files. Used by --git-pending to scope the scan
    to what you're actually working on."""
    def git(*args: str) -> list[str]:
        out = subprocess.run(
            ["git", *args], cwd=root, capture_output=True, text=True, check=True
        )
        return [ln for ln in out.stdout.splitlines() if ln.strip()]

    toplevel = Path(git("rev-parse", "--show-toplevel")[0])
    names = set(git("diff", "--name-only", "HEAD"))                  # staged + unstaged
    names |= set(git("ls-files", "--others", "--exclude-standard"))  # new untracked
    return {(toplevel / n).resolve() for n in names}


def iter_rust_files(root: Path, restrict_to: set[Path] | None = None):
    for path in root.rglob("*.rs"):
        # --git-pending restricts the scan to files with pending changes.
        if restrict_to is not None and path.resolve() not in restrict_to:
            continue
        rel_parts = path.relative_to(root).parts
        if set(rel_parts) & EXCLUDED_DIRS:
            continue
        # Skip build scripts.
        if path.name == "build.rs":
            continue
        # Skip anything whose path or file name contains "test", covers
        # `tests/`, `*_test.rs`, `test_*.rs`, `test_data/`, etc.
        rel_lower = path.relative_to(root).as_posix().lower()
        if "test" in rel_lower:
            continue
        yield path


def scan(root: Path, restrict_to: set[Path] | None = None) -> dict[Smell, list[Hit]]:
    results: dict[Smell, list[Hit]] = {s: [] for s in SMELLS}
    for path in iter_rust_files(root, restrict_to):
        try:
            ctx = FileContext.load(path)
        except OSError:
            continue
        for smell in SMELLS:
            if not smell.applies_to_file(ctx):
                continue
            for idx, line in enumerate(ctx.lines, start=1):
                if ctx.is_line_ignored(idx, line):
                    continue
                excerpt = smell.check_line(ctx, idx, line)
                if excerpt is not None:
                    results[smell].append(Hit(path=path, line_no=idx, text=excerpt))
    return results


def report(results: dict[Smell, list[Hit]], root: Path) -> int:
    total = 0
    for smell, hits in results.items():
        if not hits:
            continue
        total += len(hits)
        print(f'SMELL: "{smell.title}"')
        # Column-align the path#line prefix for readability.
        prefixes = [
            f"{hit.path.relative_to(root).as_posix()}#{hit.line_no}"
            for hit in hits
        ]
        width = max((len(p) for p in prefixes), default=0)
        for hit, prefix in zip(hits, prefixes):
            print(f"  - {prefix.ljust(width)}    {hit.text}")
        print()
    if total == 0:
        print("No smells detected.")
    else:
        print(f"Total: {total} hit(s) across {sum(1 for v in results.values() if v)} smell(s).")
    return total


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "root", nargs="?",
        help="Repo root to scan (defaults to the parent of this script).",
    )
    parser.add_argument(
        "--git-pending", action="store_true",
        help="Only scan files with pending git changes (staged, unstaged, and "
             "new untracked), so you can clean up smells just on what you're "
             "working on.",
    )
    args = parser.parse_args(argv[1:])

    root = Path(args.root).resolve() if args.root else Path(__file__).resolve().parent.parent
    if not root.is_dir():
        print(f"Not a directory: {root}", file=sys.stderr)
        return 2

    restrict_to: set[Path] | None = None
    if args.git_pending:
        try:
            restrict_to = _git_pending_files(root)
        except (OSError, subprocess.CalledProcessError) as e:
            print(f"Could not list pending git files: {e}", file=sys.stderr)
            return 2
        pending_rs = sum(1 for p in restrict_to if p.suffix == ".rs")
        print(f"# --git-pending: {pending_rs} pending .rs file(s) (test/support still skipped)\n")

    results = scan(root, restrict_to)
    hits = report(results, root)
    # Exit non-zero if anything was found, so CI can gate on it.
    return 1 if hits else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))

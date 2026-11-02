---
name: policy_checker
description: Use this skill when the user wants to audit POLICY comments in the codebase (e.g. "check policies", "run policy check", "audit POLICY comments", "are policies being followed"). Scans for `// POLICY:` / `/// POLICY:` style notes, identifies the code each policy governs, checks compliance at every relevant call site, prints a summary table, and asks before remediating.
---

# Policy Checker

This skill audits in-code POLICY notes: short rules the author has pinned next to a field,
function, struct or module, and verifies the rest of the codebase actually obeys them.

A POLICY is the one comment tag that makes a claim about code somewhere ELSE. That is what
makes it worth auditing, and what makes it rot silently.

## What a POLICY comment looks like

Any Rust comment whose first non-whitespace token after the comment marker is `POLICY:`
(case sensitive). The marker can be `//` or `///`, with 0 or more spaces before `POLICY:`.

```rust
//POLICY: Dirty flag must be set on every write operation.
// POLICY: Dirty flag must be set on every write operation.
/// POLICY: Public API, never block on this method.
```

A policy may span multiple comment lines. Continuation lines are the comment lines
immediately following the `POLICY:` line with no blank line between them, that do not
themselves start a new tag. Treat the joined text as the full policy statement.

## Step 0 - Load the manifest (incremental cache)

The skill keeps a manifest at `.claude/skills/policy_checker/manifest.json` recording, per
policy, which files have been checked and at what commit. On re-runs, unchanged files are
skipped and their previous result reused.

Load `manifest.json` if present; otherwise treat the run as a full scan. Schema:

```json
{
  "version": 1,
  "policies": {
    "<policy_hash>": {
      "text": "Dirty flag must be set on every write operation.",
      "anchor": "DocumentModel.dirty",
      "anchor_location": "src/model/document.rs:120",
      "last_scan_date": "2026-01-01",
      "last_scan_commit": "abc123def",
      "files": {
        "src/model/document.rs": {
          "last_checked_commit": "abc123def",
          "status": "OK",
          "site_count": 3
        }
      }
    }
  }
}
```

`policy_hash` identifies the policy TEXT, not its line number, so a policy that moves within
a file is still recognised. Compute it as the first 12 hex chars of `sha1(normalized_text)`,
where normalisation strips leading/trailing whitespace and collapses internal whitespace
runs to single spaces:

```bash
printf '%s' "Dirty flag must be set on every write operation." \
  | tr -s '[:space:]' ' ' | sed -e 's/^ //' -e 's/ $//' \
  | sha1sum | cut -c1-12
```

**Skip rule:** for each file in a policy's scope, run `git log -1 --format=%H -- <path>`. If
the manifest has an entry for `(policy_hash, path)` whose `last_checked_commit` equals that
SHA, and the file is clean in the working tree (`git status --porcelain -- <path>` is
empty), reuse the cached status and skip reading the file.

**Always re-check** when: the policy text changed (new hash), the anchor moved to a different
file, the file has uncommitted modifications, or the user asks for a full scan.

**Prune** entries whose `policy_hash` no longer appears in the source. Do not silently drop
them; list pruned policies in the report so the user knows the cache shrank.

At the end of the run, write the manifest back with updated `last_checked_commit`
(`git rev-parse HEAD`) and `last_scan_date` for every (policy, file) actually inspected.

## Step 1 - Find every policy

```bash
rg -n --type rust '^\s*///?\s*POLICY:' src
```

Fall back to grep if `rg` is unavailable:

```bash
grep -rnE '^[[:space:]]*///?[[:space:]]*POLICY:' --include='*.rs' src
```

Record: file path, line number, full policy text including continuation lines, and the
immediately-following code item the comment attaches to. Read about 10 lines after each hit
to identify the attached item.

## Step 2 - Determine the scope each policy governs

| Attached item        | Compliance scope                                                    |
|----------------------|----------------------------------------------------------------------|
| **Struct field**     | Every write to that field: `x.f = ...`, `x.f += ...`, `&mut x.f`, destructuring assignment, `mem::replace(&mut x.f, ...)`, `mem::take(&mut x.f)`. |
| **Function/method**  | Every call site.                                                    |
| **Struct / enum**    | Every construction site and every `impl` block.                     |
| **Module / file header** | Every item declared inside the module.                          |
| **Trait method**     | Every implementation of that method across all impls.               |

If the policy text names a *different* anchor than the item it sits beside, **the policy
text wins**. The scope is whatever the policy describes.

## Step 3 - Locate the affected sites

- **Field writes:** `rg -n '\.field\s*(=|\+=|-=|\*=|/=|%=)'` plus `rg -n '&mut\s+\w+\.field'`
  and `rg -n 'mem::(replace|take)\([^)]*\.field'`. Do not forget destructuring where the
  binding is then mutated.
- **Function calls:** `rg -n '\bfn_name\s*\('`, then filter out the definition.
- **Trait impls:** `rg -n 'impl\s+(.*\s+for\s+)?TraitName\b'`.

Prefer a single Explore subagent for large enumeration jobs, to keep the main context clean.

## Step 4 - Check compliance at each site

Apply the skip rule from step 0 first: group sites by file, and let cached files contribute
their counts without a read.

For files that need checking, read enough surrounding context to judge. Examples:

- *Policy:* "Dirty flag must be set on every write operation."
  *Check:* within ~5 lines of the write, is the corresponding flag set (or a method called
  that is documented to set it)?
- *Policy:* "Never block on this method."
  *Check:* does the call site `.await`, `block_on`, or sit in a sync hot path?
- *Policy:* "Only call from the main thread."
  *Check:* is the call inside a `spawn` or worker closure?

Mark each site **OK**, **VIOLATION**, or **UNCLEAR**. For unclear cases, note exactly what
made it ambiguous.

## Step 5 - Print the summary table

One markdown table covering every policy, ordered by file then line, with clickable links.

| Policy | Anchor | Sites | OK | Violations | Unclear | Cache |
|--------|--------|-------|----|------------|---------|-------|
| Dirty flag must be set on every write | [`document.rs:120`](src/model/document.rs#L120) - `DocumentModel.dirty` | 14 | 12 | 1 | 1 | 12 cached / 2 fresh |

Follow it with a per-policy breakdown listing each VIOLATION and UNCLEAR site as
`- [path:line](path#Lline) - reason`. Do not list OK sites individually; the count is enough.

List any pruned policies under a "Removed policies" subheading. End with a verdict line:
number of policies, number with violations, number fully compliant.

## Step 6 - Ask before remediating

**Do not auto-fix.** After the report, ask via `AskUserQuestion` whether to act. Reasonable
options: fix all violations now (showing each diff), fix one specific policy or site, file
TODO/FIXME comments at the violating sites, or report only.

If the user picks a fix, apply it one site at a time and re-run the relevant slice of step 4
to confirm.

## Notes and edge cases

- **Do not flag the policy comment itself** as a site. The comment is the rule, not a use.
- **Test files count** unless the policy says otherwise. Tests often legitimately violate an
  invariant in order to exercise it; mark those UNCLEAR rather than VIOLATION.
- **Generated code** (`build.rs` output, `target/`, `OUT_DIR`) is out of scope.
- Sibling repos outside this workspace are out of scope unless the user asks.
- If zero policies are found, say so plainly and stop. Do not invent a report.
- **The manifest is project-local and worth committing.** Mention that the first time you
  write it, and let the user decide whether it is tracked or gitignored.
- **If git is unavailable** (no `.git`, or a shallow clone), skip step 0 and full-scan every
  time. Note that in the report.

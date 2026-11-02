# Code standard

This is how we roll.

## Layout

  - Area breaks are `// ----------` and always finish on col 120.
  - The top of the file is code, the bottom of the file is tests.
  - 3 consecutive area breaks separate the top (code) from the bottom (tests).
    The 3-line thing creates a noticeable effect in the code overview widget, which
    is the point: you can see where the tests start without reading anything.
  - An area break has a comment after it naming the area:

```rust
// --------------------------------------------------------------------------------------------------------------------
// File IO code

struct FileCache { ... }
```

  - I like my code compact, iff that does not reduce readability.
  - mod.rs / lib.rs are for scaffolding only. Declaring code or test in there is not on.

## Production code

Production code is any code in a non-research module that is not a unit test, and not
in a `bin/*` used for testing.

  - Don't use `unwrap()` in production.
  - Panics are not allowed in production code. If the software does need to halt, use a
    graceful exit.
  - Don't print to stdout/stderr in production code.
  - Log shit! Especially if the code path is likely to need investigation, or if it is an
    error path, log that.
  - Don't create spam logs. If code is possibly in a high-frequency loop don't log info;
    gate the first error log, etc.
  - ABSOLUTELY no "silent fallbacks" if something is wrong. The code does not pretend it is
    doing its job.
    - A silent fallback, where the code returns something empty or best-effort, should only
      happen IFF that is an explicitly necessary thing inherent in the problem. Never
      because it prevents a larger refactor or reduces plumbing.
    - A silent fallback should generally include a logged warning. If it is more like a
      logged error, that is probably a sign it should not be a silent fallback at all.
  - All code is to be of professional quality and engineered for robustness.
  - Code should be performant unless doing so is unwarranted in the situation.

## Comments

  - A comment is almost always ONE LINE unless there is good reason.
  - More than 1 line is a trigger for a dev to stop and think.
    - Violating this creates a comment that cries wolf, and is TOXIC to the code base.
  - If code has a significant drawback or issue, flag it with a one-line comment.
  - Never remove an existing comment when editing code, even when rewriting the block. The
    author's comments are intentional memory joggers. Only remove one if the code it
    describes is being deleted as part of the current task, or if leaving it would be
    factually misleading about the new code.
  - No backticks in comments. A run of comment lines with backticks in them is machine-padded
    prose, and the smell scanner flags it.

If a fn comment needs to be "long", do it like this:

```rust
/// Super simple high level statement of what it does
///   - Brief point form notes that actually matter.
///   - Behaviour quirks and non-obvious assumptions.
/// Note: There is a wrinkle here that is very important.
```

### Comment tags

When a comment is more than a casual aside, lead with one of the project's tags so it is
greppable. Format: `// TAG: text`.

| Tag         | Meaning                                                                 |
|-------------|-------------------------------------------------------------------------|
| `TODO`      | Unfinished work that needs to be re-visited.                            |
| `FIXME`     | Known bug.                                                              |
| `WIP`       | Work (currently) In Progress, not ready for code review.                |
| `KLUDGE`    | Ugly because circumstances forced it.                                   |
| `HACK`      | Relies on an undocumented, unofficial, or unintended behaviour.         |
| `WHERE_IS`  | Followed by the name of a bookmark.                                     |
| `NOTE`      | Something that would be covered in a code handover.                     |
| `ISSUE`     | Call out an issue / ticket number, e.g. `// ISSUE: #1`.                 |
| `POLICY`    | A constraint the rest of the code must obey. Audited by the policy_checker skill. |

`// ISSUE: #N` is not a record of "this line was touched for issue N", git already knows
that. It means "this is here for a reason; you cannot change it without re-breaking #N".
Expect zero or one on a typical fix. Diagnostic logging or temporary code tied to a tracked
bug gets one, so it is easy to find and remove when the issue closes.

## Naming and types

  - 1-2 letter variable names are generally avoided except:
    - in closures etc, or
    - if it makes sense (eg: `i`, `x`, `y`).
    - `l` and `o` are never ok, they look like 1 / 0.
  - `Option<...>` is the idiomatic way to say a value is not present. Use of a
    sentinel or reserved value needs to be justified.
  - Widget abbreviations: `txt`, `lbl`, `cmb`, `lst`, `tags`, `btn`, `sb` (scrollbar),
    `grp`, `num` (numeric up/down), `sld` (numeric slider), `tab`, `dlg`.
  - Use existing vocab. Check the vocabulary in use elsewhere; don't pull in new words
    unless it is genuinely a new concept.

## Traits

  - `Debug` never dumps the contents of a data structure, only its metadata. Don't derive
    `Debug` if that would be the result.

## Output characters

Never emit non-ASCII punctuation in any output: code, comments, commit messages, chat.
Specifically no em-dash, en-dash, ellipsis, curly quotes, or NBSP. Use `-`, `--`, `...`,
`"`, `'`, and a regular space. `just vet-chars` enforces this.

Never emit emoticons or related unicode in script files. It looks tacky and makes grepping
weird.

## Dependencies

New crates are only introduced if necessary, and the crate is high quality. Prefer an
existing crate already in the tree. If a code suggestion requires a new crate, that is a
major decision requiring approval.

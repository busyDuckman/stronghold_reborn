# doco/

Deep-dive documentation. One file per subsystem, named `about_<subsystem>.md`, and every
one of them linked from CLAUDE.md under "Deep-dive docs" so it actually gets read.

The split with the other docs in this repo:

| Where              | Holds                                                                    |
|--------------------|---------------------------------------------------------------------------|
| `CLAUDE.md`        | Orientation. What this project is, its layout, its conventions, the index. |
| `code_standard.md` | How code is written here. Rules, not explanations.                        |
| `doco/about_*.md`  | How one subsystem actually works. The stuff you would say in a handover.  |
| `.plans/*.md`      | The owner's working notes. Personal space; read only when invited.        |
| `todo.md`          | The owner's scratchpad. Personal space; read only when invited.           |

A doc goes in `doco/` when a task touching that area would otherwise start with half an
hour of code reading. Write it once, link it from CLAUDE.md, and every future session
starts warm.

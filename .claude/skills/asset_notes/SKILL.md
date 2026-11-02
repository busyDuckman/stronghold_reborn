---
name: asset_notes
description: The STRONG.DAT asset browser is a shared channel between you and the owner. They can say "I left some notes about 1025 on the website" and you go read them; you can hand back a localhost:8731 URL so they can click through and see the thing you are asking about. Read this when the owner mentions the website, the asset browser or the sprite viewer, says they left notes, says "look at entry N", or whenever you are about to describe a sprite, a palette, an animation or a sound in prose. Read it before touching reversing/notes/dat_notes.json or dat_names.tsv by any means.
---

# The asset browser is a shared channel, both ways

`just dat-web` serves a browser for STRONG.DAT at `http://127.0.0.1:8731`. It shows a sprite
under any palette, plays a run of entries as an animation, plays sampled audio, scrubs the
FLI sequences, and prints hex with the header fields named. The owner works in it. So can
you, at one remove.

It exists because most questions about this container are settled by looking or listening,
and neither of you can put a sprite into a chat message.

## They point you at it

> I left some notes about 1025 on the website

That is a complete instruction. Go and read them:

    just dat-notes list --entry 1025

They will not usually give you a URL, and they should not have to. "On the website", "in the
browser", "I left a note on that sprite" all mean the same thing: there is something in
`dat_notes.json` you have not read.

## You point them at it

This is the half that is easy to forget. When something needs their eyes, hand over a link
rather than a paragraph. The fragment is the entry index:

> entries 4 through 11 look like one animation to me. http://127.0.0.1:8731/#4 - set
> animate to 8 frames and press play, is that a gem spinning?

> http://127.0.0.1:8731/#1371 under palette 741 comes out green, under 1451 it comes out
> with two magenta pixels. Does either look right to you?

Do that whenever you would otherwise be describing pixels, colours, timing or a sound in
prose. You cannot see the asset and they can, so the useful move is to put it in front of
them and ask, not to narrate your guess about it.

Things you can send them to do there: step entries with the left and right arrows, change
the palette with the box at the top or the `[` and `]` keys, "render under all 56" to see
one sprite under every palette at once, animate N consecutive frames at a chosen rate, play
a sound, switch the display filter between flat, xBR and CRT, or read the hex with its
header fields labelled.

Check the page is actually up before sending a link, or the invitation is a dead end:

    curl -sf localhost:8731/api/catalog >/dev/null || just dat-web

## Reading the notes

    just dat-notes list --entry 1025    # one thread
    just dat-notes list                 # everything
    just dat-notes open                 # threads the owner has not closed
    just dat-notes export               # the whole log as markdown

## Do not reply unless you are asked to

This is the part that matters, and it is not obvious.

**The notes are the owner's record, not a conversation with you.** Most are a memo to self:
something noticed in passing, to be settled later when the decompile can answer it. A thread
that ends on the owner's note is an open loose end of theirs. Adding a reply closes it, so a
note left as a reminder stops looking like a reminder, and a question they meant to return to
in six months looks answered and dealt with.

So `just dat-notes open` is not an inbox and not a queue to clear. Read it for context.
Leave it alone otherwise.

Write a note only when the owner asks you to record something, and then say the thing and
stop. Do not acknowledge, do not summarise what they already wrote, do not close a loop that
was not open. When you have an answer they did not ask for, put it in chat with a link to
the entry: chat scrolls away harmlessly, and the link lets them go and look if they care.

    just dat-note-add 1025 "the 16 sample bytes are all 127"

## Never delete the owner's notes

`just dat-notes drop <id>` is theirs to run, not yours. A note records a judgement made by
looking or listening at something you cannot re-derive from the container: nobody gets
"that one sounds like a dragon" out of the bytes. Deleting one destroys the only copy.

Removing a note of your own that they have told you is noise is a different thing and is
fine. That is cleaning up after yourself.

The same care applies to `reversing/notes/dat_names.tsv`, and to editing either file by hand
instead of through the recipes.

## Looking at the entry they mean

The browser's API answers the same questions from a terminal:

    curl -s localhost:8731/api/entry/1025 | jq     # everything about one entry
    just dat-list | sed -n '1026p'                 # index, offset, size, kind

If nothing answers, the server is not running: `just dat-web`. The owner sometimes reports
it as crashed when it has simply been stopped.

## Filenames

`dat_names.tsv` is `index<TAB>name`, written by the owner when they decide what an entry is.
It overrides everything inferred, and it is the only source of names that is not a guess.
`just dat-names` shows the inferred ones, graded: `confirmed` bindings are checked pairings,
`candidate` is a positional lineup that is right for a while and then drifts, `user` is that
file.

## The standing questions are not yours

Which palette belongs to which asset, and which source file produced which sprites, are both
unresolved and both come out of the decompile. Do not go hunting them off your own bat, and
do not write speculation into the notes or into `doco/`: it becomes an assumption that later
work inherits without knowing where it came from. Help if asked. Otherwise answer what was
actually asked and leave the rest alone.

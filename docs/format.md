# Files: .gph and .gphz

## .gph

Plain UTF-8 text, the diagram language (see `plan.md`). This is the source
of truth: diffable, hand-editable, friendly to git. A diagram without
pictures is always a `.gph`.

Pictures can be linked from a `.gph` by path, relative to the file:

```graphing
logo: image { src: "art/logo.png", fit: contain }
```

`fit` is `contain` (default, letterboxed), `cover` (fills, cropped) or `fill`
(stretched). Any node with `src` shows the picture; the `image` shape is the
plain one.

Formats: PNG, JPEG, GIF, WebP, AVIF, BMP and SVG. Animated GIF, WebP and
AVIF play (at most 20 frames a second, only while the window is in front);
Settings > Canvas > Moving Pictures makes them play always, only under the
pointer, or never. Decoding matches notesy's, limits included: an animation
longer than 300 frames or larger than about 64 MB of frames shows its first
frame, and pictures are kept at most 1600 px on the long side.

Animation steps (`animate { step "..." { show a  flow a -> b } }`) live in
the text too; see `animation.md`.

A diagram can point at another with a link shape:

```graphing
auth: ref { src: "auth.gph" }           # relative to this file's folder
```

It draws as a card titled after the linked diagram, with a miniature of it
that follows the file as it changes. Double-click opens it; exports draw the
miniature too. A missing file says so on the card.

A `.gphz` carries its links along: saving or packing one stores a snapshot
of each linked diagram's text (`linked/<src>`, refreshed on every save).
Where the linked file exists the link stays live; where it does not (the
package was shared), the card shows the snapshot and double-click opens it
as an unsaved copy. Snapshots go when no link names them. `unpack` puts them
in `assets/linked/`, and `pack` takes them back.

Dropping `.gph` files (or mermaid, draw.io, SysML v2, Visio files) on a
canvas asks whether to open them, insert their contents (ids prefixed by the
file's name, inside a group titled after it, one undo step) or link to them.
Dropped anywhere else in the window, they open.

## .gphz

A diagram with its pictures inside, in one compact file. The text inside is
the same `.gph`, with pictures referenced as `asset:<name>`:

```graphing
logo: image { src: "asset:logo-20639fdb.png" }
```

Not a zip. A 6-byte header, then one record per entry:

| Field | Size | Meaning |
| --- | --- | --- |
| magic | 4 | `GPHZ` |
| version | 1 | `1` |
| flags | 1 | `0` |
| *per record:* | | |
| kind | 1 | 0 document, 1 meta (JSON), 2 asset |
| codec | 1 | 0 stored, 1 zstd |
| name_len | 2 | little endian |
| name | name_len | UTF-8 |
| raw_len | 4 | size after decoding |
| data_len | 4 | size as stored |
| hash | 32 | BLAKE3 of the decoded bytes |
| data | data_len | |

Why not zip: the text is zstd-compressed (smaller and faster than deflate);
pictures already compressed (PNG, JPEG, WebP, GIF) are stored as they are
instead of being deflated again for nothing; assets are named by content
hash, so the same picture is kept once however often it is used; every
entry is checked on read; there is no central directory or per-file
timestamps. Assets the text no longer references are dropped on save.

Readers skip record kinds they do not know, so later versions can add
entries (thumbnails, say) without breaking older builds.

## In the app

- Adding a picture to an unsaved diagram or a `.gphz` keeps it inside.
- Adding one to a plain `.gph` asks: save as `<name>.gphz`, or link the
  file where it is (pasted pictures have no file, so only the package).
- Saving a diagram that has pictures always writes a `.gphz`.
- Pictures come in by Edit > Insert Image, dragging files onto the canvas,
  or pasting.

## CLI

```
graphing pack diagram.gph -o diagram.gphz   # linked pictures move inside
graphing unpack diagram.gphz -o folder/     # folder/diagram.gph + folder/assets/
graphing pack folder/ -o diagram.gphz       # back again
graphing info diagram.gphz                  # what is inside, sizes
graphing render diagram.gphz -o out.svg     # pictures embedded in SVG/PNG
```

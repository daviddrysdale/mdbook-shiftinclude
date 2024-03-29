# mdbook-shiftinclude

A preprocessor for [mdbook](https://github.com/rust-lang/mdBook) for including portions of files,
but with the contents of the file shifted.

The `{{#shiftinclude }}` command extends the
[syntax](https://rust-lang.github.io/mdBook/format/mdbook.html#including-files) of the normal `{{#include }}` command to
include a shift indicator.  This is followed by a colon, and then the normal `include` syntax follows.

- A number, indicating the amount to shift.
  - A positive number shifts right by prepending that number of spaces to each line.
  - A negative number shifts left by removing that number of characters from the start of
    each line (regardless of whether they are spaces or not!).
- `auto`, which indicates that any block of whitespace that is common to all (non-empty) lines
  in the included text will be removed.

So for an input file `somefile.txt`:

```text
  Indent
     More Indent
  Back
```

The following outputs are possible:

- `{{#shiftinclude auto:somefile.txt}` gives
   ```text
   Indent
      More Indent
   Back
   ```
- `{{#shiftinclude 2:somefile.txt}` gives
   ```text
       Indent
          More Indent
       Back
   ```
- `{{#shiftinclude -2:somefile.txt}` gives
   ```text
   Indent
      More Indent
   Back
   ```
- `{{#shiftinclude -4:somefile.txt}` gives
   ```text
   dent
    More Indent
   ck
   ```

## Installation

To use, install the tool

```sh
cargo install mdbook-shiftinclude
```

and add it as a preprocessor in `book.toml`:

```toml
[preprocessor.shiftinclude]
```

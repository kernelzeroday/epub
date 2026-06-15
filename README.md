# epub

Non-interactive EPUB text extraction CLI. Extract chapters to stdout for piping to `say`, `llm`, or any other tool.

## Install

```
cargo install --git https://github.com/kernelzeroday/epub.git
```

## Usage

```
epub book.epub                  # dump entire book
epub book.epub -c 3             # extract chapter 3
epub book.epub -c 1-5           # extract chapters 1 through 5
epub book.epub -c 1,3,7-10      # mixed selection
epub book.epub -l               # list chapters with indices
epub book.epub -i               # show metadata
```

Chapter indices from `-c` correspond to the numbers shown by `-l`.

### Pipe to macOS text-to-speech

```
epub book.epub -c 4 | say -v Alex
```

### Pipe to an LLM

```
epub book.epub -c 4 | llm "summarize this chapter"
```

## How it works

Parses the EPUB's OPF spine and NCX table of contents directly using `zip` and `quick-xml`. When you select a chapter with `-c`, it resolves the TOC entry to the correct spine item(s) and extracts plain text with HTML stripped. Image-only chapters (maps, illustrations) produce a diagnostic on stderr.

## License

MIT

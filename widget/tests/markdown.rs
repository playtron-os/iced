//! Tests for the [`iced_widget::markdown`] parser: the incremental
//! [`Content::push_str`] method must converge to the one-shot
//! [`Content::parse`] result, whatever the chunks are.
//!
//! [`Content::push_str`]: iced_widget::markdown::Content::push_str
//! [`Content::parse`]: iced_widget::markdown::Content::parse
use iced_widget::markdown::Content;

/// A long list of bullets.
#[test]
fn bullets() {
    let full = (0..50).map(|i| format!("- item {i}\n")).collect::<String>();
    assert_converges(&full, "bullets");
}

/// A long ordered list.
#[test]
fn ordered_list() {
    let full = (1..=30)
        .map(|i| format!("{i}. step {i}\n"))
        .collect::<String>();
    assert_converges(&full, "ordered list");
}

/// A task list.
#[test]
fn task_list() {
    let full = "- [ ] a\n- [x] b\n- [ ] c\n- [x] d\n- [ ] e\n";
    assert_converges(full, "task list");
}

/// A nested list.
#[test]
fn nested_list() {
    let full = "- a\n  - b\n  - c\n- d\n  - e\n- f\n";
    assert_converges(full, "nested list");
}

/// A loose list (blank lines between bullets).
#[test]
fn loose_list() {
    let full = "- a\n\n- b\n\n- c\n";
    assert_converges(full, "loose list");
}

/// Bullets with multiple paragraphs.
#[test]
fn multi_paragraph_bullets() {
    let full = "- a\n\n  more a\n\n- b\n\n  more b\n";
    assert_converges(full, "multi paragraph bullets");
}

/// A list surrounded by other items.
#[test]
fn list_surrounded_by_other_items() {
    let full = "# Title\n\nintro\n\n- a\n- b\n\nafter\n\n- c\n";
    assert_converges(full, "list surrounded by other items");
}

/// A code block followed by a list.
#[test]
fn code_block_and_list() {
    let full = "```\ncode\n```\n\n- a\n- b\n";
    assert_converges(full, "code block and list");
}

/// A reference whose definition is swallowed by a metadata block must
/// stop resolving the links that were resolved with it while the block
/// was still open.
#[test]
fn reference_swallowed_by_metadata_block() {
    let full = "para\n\n- [x][ref]\n---\nkey: value\n| a | b |\n| - | - |\n| 1 | 2 |\n\n2. two [y][ref]\n![alt with [x][ref]](https://img.com/i.png)\n\n+++\nkey: value\n+++\n\n[ref]: https://changed.com\n\n---";
    assert_converges(full, "reference swallowed by a metadata block");
}

/// A reference defined both inside a swallowed metadata block and
/// outside it resolves to the outside definition, like the one-shot
/// parse does.
#[test]
fn reference_first_defined_in_swallowed_block() {
    let full =
        "a [x][ref]\n\n---\n[ref]: https://in_block.com\n---\n\n[ref]: https://outside.com\n";
    assert_converges(full, "reference first defined in a swallowed block");
}

/// A list inside a quote, followed by another list.
#[test]
fn list_in_quote() {
    let full = "> - a\n> - b\n\n- c\n";
    assert_converges(full, "list in quote");
}

/// A bullet that looks like a setext heading underline.
#[test]
fn setext_like_bullet() {
    let full = "- a\n  ---\n- b\n";
    assert_converges(full, "setext like bullet");
}

/// A bullet containing a link and an image.
#[test]
fn bullet_with_link_and_image() {
    let full = "- [link](https://a.com) ![img](https://b.com/i.png)\n- b\n";
    assert_converges(full, "bullet with link and image");
}

/// A bullet ending with a pipe (table-ish).
#[test]
fn bullet_ending_with_pipe() {
    let full = "- foo |\n- bar\n";
    assert_converges(full, "bullet ending with pipe");
}

/// A single very long bullet.
#[test]
fn long_single_bullet() {
    let full = format!("- {}\n", "word ".repeat(500).trim_end());
    assert_converges(&full, "long single bullet");
}

/// A broken reference link, resolved once the reference (defined
/// after the list) arrives.
#[test]
fn broken_reference_link_resolved_later() {
    let full = "- a [x][ref]\n- b [y][ref]\n- c\n\n[ref]: https://example.com\n";
    assert_converges(full, "broken reference link resolved later");
}

/// A broken reference link, resolved between two bullets of the same
/// list.
#[test]
fn broken_reference_link_resolved_between_bullets() {
    let full = "- a [x][ref]\n\n[ref]: https://example.com\n\n- b [y][ref]\n";
    assert_converges(full, "broken reference link resolved between bullets");
}

/// A table, streamed word by word.
#[test]
fn table_word_by_word() {
    let full = "| a | b |\n| - | - |\n| 1 | 2 |\n| 3 | 4 |\n\nDone.\n";
    assert_converges(full, "table word by word");
}

/// An image and a reference link on the same line; the image must not
/// be duplicated as the line grows.
#[test]
fn image_and_link_on_same_line() {
    let full = "prev\n\n![alt](https://img.com/i.png) [x][ref]\n\ntext\n";
    assert_converges(full, "image and link on same line");
}

/// Several images and a paragraph on the same line; the images and
/// the paragraph share the same source region, and the images must
/// not be duplicated as the line grows.
#[test]
fn images_on_same_line() {
    let full = "a\n\n![i1](https://u1.com/1.png) ![i2](https://u2.com/2.png) t\n\nb\n";
    assert_converges(full, "images on the same line");
}

/// A line with only images, followed by more content.
#[test]
fn image_only_line() {
    let full = "a\n\n![i1](https://u1.com/1.png) ![i2](https://u2.com/2.png)\n\nb\n";
    assert_converges(full, "image-only line");
}

/// Text followed by an image on the same line; the text must not be
/// absorbed into the image's alt.
#[test]
fn text_then_image() {
    let full = "hello ![img](https://u1.com/1.png)\n\nmore\n";
    assert_converges(full, "text then image");
}

/// A reference defined before it is used.
#[test]
fn reference_before_use() {
    let full = "[ref]: https://example.com\n\n- [x][ref]\n- [y][ref]\n";
    assert_converges(full, "reference before use");
}

/// A metadata block after content should not break the flow.
#[test]
fn metadata_after_content() {
    let full = "- a\n\n+++\nmeta: 1\n+++\n\n- b\n";
    assert_converges(full, "metadata after content");
}

/// Many constructs at once, streamed block by block, word by word and
/// char by char.
#[test]
fn mixed_document() {
    let parts = [
        "# Title\n\n",
        "intro with **bold** and *emphasis* and `code`\n\n",
        "- a\n",
        "- b [link](https://a.com)\n",
        "  - nested\n",
        "- [ ] task\n",
        "- [x] done\n\n",
        "1. one\n",
        "2. two\n\n",
        "> quote\n",
        "> - in a quote\n\n",
        "```\ncode block\n```\n\n",
        "![alt](https://img.com/i.png) [x][ref]\n\n",
        "text\n\n",
        "[ref]: https://example.com\n\n",
        "- c\n",
        "- d\n\n",
        "| a | b |\n| - | - |\n| 1 | 2 |\n",
        "\n\nafter\n",
    ];
    let full = parts.concat();

    assert_converges(&full, "mixed document");
}

/// Pushing new bullets to a long list must stay cheap: `push_str`
/// re-parses only the last bullet, so the time per push does not grow
/// with the list.
#[test]
fn pushing_to_a_long_list_stays_fast() {
    let mut c = Content::new();
    let bullet = |i| format!("- item {i} with some **text** and a `code`\n");

    for i in 0..800 {
        c.push_str(&bullet(i));
    }

    // The last 200 pushes, as a fraction of the total, must be cheap:
    // with an O(n) re-parse per push they would take most of the time.
    let start = std::time::Instant::now();
    for i in 800..1000 {
        c.push_str(&bullet(i));
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs_f64() < 2.0,
        "the last 200 pushes took {elapsed:?}, which suggests that the \
         whole list is re-parsed on every push"
    );
    assert_eq!(c.items().len(), 1);
}

#[test]
fn reference_resolved_before_next_item() {
    let full = "- a [x][ref]\n\n[ref]: https://example.com\n\npara\n";
    assert_converges(full, "reference resolved before the next item");
}

#[test]
fn duplicate_reference_definitions() {
    // The first definition wins, like in CommonMark; the second
    // definition is still growing (not terminated) when the push
    // happens.
    let full = "- [x][ref]\n\n[ref]: https://one.com\n\npara\n\n[ref]: https://two.com\n";
    assert_converges(full, "duplicate reference definitions");
}

#[test]
fn broken_link_in_image_alt() {
    let full = "before\n\n![alt with [x][ref]](https://img.com/i.png)\n\n[ref]: https://example.com\n\nafter\n";
    assert_converges(full, "broken link in an image alt");
}

#[test]
fn broken_link_in_image_alt_with_trailing_text() {
    // The section is created for the image before its paragraph is
    // complete; the re-parse range must then span the whole line.
    let full =
        "![alt [x][ref]](https://img.com/i.png) trailing\n\n[ref]: https://example.com\n\nafter\n";
    assert_converges(full, "broken link in an image alt, with trailing text");
}

/// A rule directly after a list item, with no blank line in between:
/// the rule ends the list, and the incremental parse must keep both
/// the list and the rule.
#[test]
fn list_then_rule_without_blank_line() {
    let full = "[other]: https://other.com\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n\
         [ref]: https://example.com\n\n- a\n---\n\n";
    assert_converges(full, "list then rule without blank line");
}

/// A reference definition directly after a list line, with no blank
/// line in between, is absorbed into the bullet (lazy continuation);
/// the list that follows must not be lost.
#[test]
fn reference_then_list_without_blank_line() {
    let full =
        "2. two\n[ref]: https://changed.com\n\n\n- a\n| a | b |\n| - | - |\n| 1 | 2 |\n\n---\n\n";
    assert_converges(full, "reference then list without blank line");
}

/// A reference definition absorbed into a quoted bullet (lazy
/// continuation) embeds a link to the label it defines; that link
/// must resolve with the one-shot semantics: the first definition of
/// the label wins, like in CommonMark.
#[test]
fn absorbed_reference_resolves_to_first_definition() {
    let full = "[ref]: https://example.com\n\n> - in quote\n[ref]: https://changed.com\n\n\
         [ref]: https://changed.com\n\n> - in quote\n> - in quote\n- b [y][ref]\n\n";
    assert_converges(full, "absorbed reference resolves to the first definition");
}

/// A list that spans the start of the re-parse window: while its
/// last line is being pushed, it is a lone paragraph that closes the
/// list; when it becomes a list item, the list must not be split in
/// two.
#[test]
fn list_spanning_the_reparse_window_is_not_split() {
    assert_converges(
        "2. two\n\n2. two\n",
        "loose list across the re-parse window",
    );
    assert_converges(
        "2. two\n2. two\n\n2. two\n",
        "list, loose list, across the re-parse window",
    );
}

/// A metadata block at the start of the stream is swallowed by the
/// parser once it is closed; the first line is a tentative rule
/// until then, so the whole block must be re-parsed as it grows.
#[test]
fn leading_metadata_block_is_swallowed() {
    assert_converges("---\nkey: value\n---\n\nmore\n", "a closed metadata block");
    assert_converges(
        "---\nkey: value\n...\n\nmore\n",
        "a metadata block closed by ellipsis",
    );
    assert_converges(
        "+++\nkey: value\n+++\n\nmore\n",
        "a plus-delimited metadata block",
    );
    // Not a metadata block: the second line is blank, so the first
    // line is a rule
    assert_converges("---\n\nmore\n", "a leading rule");
    // Not closed: the `---` is a rule and the block stays open
    assert_converges(
        "---\nkey: value\n--- x\n\nmore\n",
        "an unclosed metadata block",
    );
}

/// A metadata block in the middle of the stream is swallowed by the
/// parser once it is closed, too: its opening line is a tentative
/// rule until then, so the whole block must be re-parsed from where
/// it starts as it grows.
#[test]
fn mid_metadata_block_is_swallowed() {
    // After a blank line, and after a list item (no blank line)
    assert_converges(
        "para\n\n---\nkey: value\n---\n\nmore\n",
        "a metadata block after a blank line",
    );
    assert_converges(
        "- x\n---\nkey: value\n---\n\nmore\n",
        "a metadata block after a list item",
    );
    assert_converges(
        "- x\n---\nkey: value\n...\n\nmore\n",
        "a metadata block closed by ellipsis",
    );
    assert_converges(
        "para\n\n+++\nkey: value\n+++\n\nmore\n",
        "a plus-delimited metadata block",
    );
    // A list follows the block: the list must not be merged with the
    // tentative rule
    assert_converges(
        "- a\n- b\n\n---\nkey: value\n---\n\n- c\n",
        "a metadata block between two lists",
    );
}

/// Inline TeX math, set as Unicode text, stays one span as it grows.
#[test]
fn inline_math() {
    let full = "The area is $\\pi r^2$ and $e^{i\\pi} + 1 = 0$, roughly.\n\nNext\n";
    assert_converges(full, "inline math");
}

/// Display math, on its own lines.
#[test]
fn display_math() {
    let full = "Sum:\n\n$$\n\\sum_{i=1}^{n} i = \\frac{n(n+1)}{2}\n$$\n\nafter\n";
    assert_converges(full, "display math");
}

/// A `$` before a digit closes no math, so prices stay as written
/// whether the closing digit has arrived yet or not.
#[test]
fn money_is_not_math() {
    let full = "It costs $5-$10 today, or $x$ dollars.\n\n- $3 to $4\n";
    assert_converges(full, "money is not math");
}

/// Math inside the other blocks: list bullets, a quote and a table.
#[test]
fn math_in_blocks() {
    let full = "- $x^2$\n- $\\alpha$ and $\\beta$\n\n> $\\sqrt{2}$ is irrational\n\n\
                | a | b |\n| - | - |\n| $x$ | $y_1$ |\n\ndone\n";
    assert_converges(full, "math in blocks");
}

/// A highlighted code block re-parsed line by line keeps the
/// highlighter state of the lines before it (the string spans lines).
#[test]
fn highlighted_code_block() {
    let full = "```rust\nfn main() {\n    let s = \"a\nb\";\n    let n = 1;\n}\n```\n\nafter\n";
    assert_converges(full, "highlighted code block");
}

/// A code line is highlighted from where the line before it left off,
/// so a string that spans two lines keeps its color on the second.
#[cfg(feature = "highlighter")]
#[test]
fn highlight_continues_across_code_lines() {
    use iced_widget::core::Theme;
    use iced_widget::markdown::{Item, Style};

    let content = Content::parse("```rust\nlet s = \"a\nb\";\n```\n");
    let Some(Item::CodeBlock { lines, .. }) = content.items().first() else {
        panic!("expected a code block: {:?}", content.items());
    };

    let style = Style::from(Theme::Light);
    let first = lines[0].spans(style);
    let second = lines[1].spans(style);

    let string = first.last().expect("the string's first line");
    let continued = second.first().expect("the string's second line");

    assert_eq!(string.text, "a");
    assert!(continued.text.starts_with('b'));
    assert_eq!(continued.color, string.color);
}

/// Fuzzes the convergence of the incremental parse over a corpus of
/// generated documents, with the three chunking schemes, checking the
/// one-shot equivalence after every push.
#[test]
fn fuzz() {
    /// The blocks from which the fuzzed documents are generated.
    const BLOCKS: &[&str] = &[
        "para\n\n",
        "text [x][ref] more\n\n",
        "- a\n",
        "- a\n- b\n",
        "- a [x][ref]\n",
        "- [x][ref]\n",
        "- b [y][ref]\n\n",
        "2. two\n",
        "2. two [y][ref]\n",
        "- [ ] task\n",
        "> - in quote\n",
        "> quote\n\n",
        "| a | b |\n| - | - |\n| 1 | 2 |\n\n",
        "| a | b |\n| - | - |\n| 1 | 2 | [x][ref] |\n\n",
        "[ref]: https://example.com\n\n",
        "[ref]: https://changed.com\n\n",
        "[other]: https://other.com\n\n",
        "[x]: https://x.com\n\n",
        "![alt](https://img.com/i.png)\n\n",
        "![alt with [x][ref]](https://img.com/i.png)\n\n",
        "before ![alt [x][ref]](https://img.com/i.png) trailing\n\n",
        "---\n\n",
        "---\nkey: value\n---\n\n",
        "+++\nkey: value\n+++\n\n",
        "---\nkey: value\n...\n\n",
        "---\nkey: value\n",
        "# heading\n\n",
        "## setext\n---\n\n",
        "```\ncode\n```\n\n",
        "inline $x^2$ math\n\n",
        "$$\n\\frac{a}{b}\n$$\n\n",
        "- $\\alpha$\n",
        "costs $5-$10\n\n",
        "\n",
    ];

    /// A deterministic pseudo-random generator.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as usize
        }
    }

    /// The documents to fuzz: the four convergence repros, and a
    /// deterministic corpus of documents generated from `BLOCKS`.
    fn corpus() -> impl Iterator<Item = String> {
        const REPROS: &[&str] = &[
            "[other]: https://other.com\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\n\
         [ref]: https://example.com\n\n- a\n---\n\n",
            "2. two\n[ref]: https://changed.com\n\n\n- a\n| a | b |\n| - | - |\n| 1 | 2 |\n\n---\n\n",
            "[ref]: https://example.com\n\n> - in quote\n[ref]: https://changed.com\n\n\
         [ref]: https://changed.com\n\n> - in quote\n> - in quote\n- b [y][ref]\n\n",
            // A metadata block at the start of the document
            "---\nkey: value\n---\n\npara\n\n- a\n",
            // A metadata block in the middle of the document, after a
            // blank line and after a list item (no blank line), closed
            // and unclosed
            "para\n\n---\nkey: value\n---\n\nmore\n\n- a\n",
            "- x\n---\nkey\n---\npara\n",
            "- x\n---\nkey\n...\nmore\n",
        ];

        let mut rng = Rng(0x5EED);

        REPROS
            .iter()
            .copied()
            .map(str::to_owned)
            .chain((0..600).map(move |_| {
                let mut doc = String::new();

                for _ in 0..4 + rng.next() % 7 {
                    doc.push_str(BLOCKS[rng.next() % BLOCKS.len()]);
                }

                doc
            }))
    }

    for doc in corpus() {
        assert_converges(&doc, "fuzzed");
    }
}

/// Asserts that parsing `full` incrementally, with different chunking
/// schemes, converges to the one-shot parse.
fn assert_converges(full: &str, label: &str) {
    let word_by_word = full.split_inclusive(' ').map(str::to_owned).collect();
    let char_by_char = full.chars().map(|c| c.to_string()).collect();

    let three_char_chunks: Vec<_> = {
        let mut i = 0;

        std::iter::from_fn(|| {
            if i >= full.len() {
                return None;
            }

            let end = (i + 3).min(full.len());
            let chunk = full[i..end].to_owned();
            i = end;

            Some(chunk)
        })
        .collect()
    };

    for (chunks, name) in [
        (word_by_word, "word-by-word"),
        (char_by_char, "char-by-char"),
        (three_char_chunks, "3-char chunks"),
    ] {
        let mut stream = Content::new();

        for chunk in chunks {
            stream.push_str(&chunk);

            let one_shot = Content::parse(stream.raw());

            assert_eq!(
                format!("{:?}", stream.items()),
                format!("{:?}", one_shot.items()),
                "{label}: incremental ({name}) should converge to one-shot\n\n{raw}",
                raw = stream.raw(),
            );

            assert_eq!(
                stream.images(),
                one_shot.images(),
                "{label}: images should converge ({name})\n\n{raw}",
                raw = stream.raw(),
            );
        }
    }
}
